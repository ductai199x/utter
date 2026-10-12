"""Export-time rewrite of the Parakeet encoder's convolutions as matrix multiplies and
shifted multiply-adds, so the ONNX graph has no Conv nodes.

Why: onnxruntime's CUDA Conv redoes its cuDNN setup whenever the input shape changes, and
every utterance has a new length. Measured on the RTX 3090 Ti (onnxruntime's profiler, 2026-10-09):
the encoder's Conv nodes take 2.3 ms on a repeated length and about 42 ms on a new one. The
rewritten modules compute the same function (checked against the originals by
`utter-speech check`), with only MatMul and elementwise nodes.

Rewrites, for one clip (batch 1, no mask; the export takes exactly the valid frames):
- conformer convolution module: pointwise conv (k=1) -> linear; GLU on the channel axis;
  depthwise conv (k=9) with the eval-mode BatchNorm folded into it, kept as a Conv on a
  fixed-length input (ConvModuleFixedConv); SiLU; pointwise conv -> linear. Works in [B, T, C]
  throughout.
- subsampling: the 3x3 stride-2 convs (the first, single input channel, and the depthwise
  ones) -> 9 strided slices weighted and summed; the 1x1 convs -> linear over channels.
"""

from __future__ import annotations

import torch
import torch.nn.functional as F
from torch import nn


class ConvModuleFixedConv(nn.Module):
    """The conformer convolution module with the pointwise convs as linear maps and the
    depthwise conv kept as a real Conv whose input length never changes: the input is
    zero-padded to `max_frames` (plus the kernel's half-width on both sides), convolved
    without padding, and sliced back to the clip's length. Exact: a kernel of half-width p
    reads at most p frames past the clip's end, and those are zeros either way, as with the
    original's SAME padding. cuDNN then sets the convolution up once, not per utterance."""

    def __init__(self, m, max_frames: int) -> None:
        super().__init__()
        self.w1 = nn.Parameter(m.pointwise_conv1.weight[:, :, 0].clone(), requires_grad=False)
        self.b1 = (
            None
            if m.pointwise_conv1.bias is None
            else nn.Parameter(m.pointwise_conv1.bias.clone(), requires_grad=False)
        )
        bn = m.norm
        scale = bn.weight / torch.sqrt(bn.running_var + bn.eps)
        b = m.depthwise_conv.bias if m.depthwise_conv.bias is not None else torch.zeros_like(scale)
        self.dw = nn.Parameter(
            (m.depthwise_conv.weight * scale[:, None, None]).clone(), requires_grad=False
        )  # [C, 1, K]
        self.dw_b = nn.Parameter(
            ((b - bn.running_mean) * scale + bn.bias).clone(), requires_grad=False
        )
        self.pad = m.padding
        self.max_frames = max_frames
        self.act = m.activation
        self.w2 = nn.Parameter(m.pointwise_conv2.weight[:, :, 0].clone(), requires_grad=False)
        self.b2 = (
            None
            if m.pointwise_conv2.bias is None
            else nn.Parameter(m.pointwise_conv2.bias.clone(), requires_grad=False)
        )

    def forward(self, x: torch.Tensor, attention_mask=None) -> torch.Tensor:  # x: [B, T, C]
        x = F.linear(x, self.w1, self.b1)
        a, g = x.chunk(2, dim=-1)
        x = (a * torch.sigmoid(g)).transpose(1, 2)  # [B, C, T]
        t = x.shape[2]
        xp = F.pad(x, (self.pad, self.max_frames - t + self.pad))  # [B, C, max_frames + 2p], fixed
        y = F.conv1d(xp, self.dw, self.dw_b, groups=self.dw.shape[0])[:, :, :t]
        return F.linear(self.act(y.transpose(1, 2)), self.w2, self.b2)


def _strided_taps(
    xp: torch.Tensor, t_out: torch.Tensor, f_out: torch.Tensor, di: int, dj: int
) -> torch.Tensor:
    """xp[..., di + 2i, dj + 2j] for i < t_out, j < f_out (a stride-2 3x3 window's tap)."""
    return xp[..., di : di + 2 * t_out - 1 : 2, dj : dj + 2 * f_out - 1 : 2]


class SubsamplingNoConv(nn.Module):
    """ParakeetEncoderSubsamplingConv2D for 3x3 stride-2 convs with padding 1, as in
    Parakeet TDT 0.6B: [conv(1->C), ReLU, (depthwise conv, pointwise conv, ReLU) x (n - 1)]."""

    def __init__(self, s) -> None:
        super().__init__()
        assert s.kernel_size == 3 and s.stride == 2 and s.padding == 1, "rewrite assumes 3x3/2/1"
        convs = [layer for layer in s.layers if isinstance(layer, nn.Conv2d)]
        first = convs[0]
        self.first_w = nn.Parameter(first.weight[:, 0].clone(), requires_grad=False)  # [C, 3, 3]
        self.first_b = nn.Parameter(first.bias.clone(), requires_grad=False)
        self.dw_w, self.dw_b, self.pw_w, self.pw_b = (
            nn.ParameterList(),
            nn.ParameterList(),
            nn.ParameterList(),
            nn.ParameterList(),
        )
        for dw, pw in zip(convs[1::2], convs[2::2], strict=True):
            self.dw_w.append(
                nn.Parameter(dw.weight[:, 0].clone(), requires_grad=False)
            )  # [C, 3, 3]
            self.dw_b.append(nn.Parameter(dw.bias.clone(), requires_grad=False))
            self.pw_w.append(
                nn.Parameter(pw.weight[:, :, 0, 0].clone(), requires_grad=False)
            )  # [C, C]
            self.pw_b.append(nn.Parameter(pw.bias.clone(), requires_grad=False))
        self.linear = s.linear

    @staticmethod
    def _out(n: torch.Tensor) -> torch.Tensor:
        return (n + 2 - 3) // 2 + 1

    def forward(self, features: torch.Tensor, attention_mask=None) -> torch.Tensor:  # [B, T, F]
        t, f = features.shape[1], features.shape[2]
        t1, f1 = self._out(torch.tensor(t)), self._out(torch.tensor(f))
        xp = F.pad(features, (1, 1, 1, 1))  # [B, T+2, F+2]
        x = self.first_b[None, :, None, None]
        for di in range(3):
            for dj in range(3):
                x = (
                    x
                    + _strided_taps(xp, t1, f1, di, dj)[:, None]
                    * self.first_w[None, :, di, dj, None, None]
                )
        x = torch.relu(x)  # [B, C, T1, F1]
        for dw_w, dw_b, pw_w, pw_b in zip(self.dw_w, self.dw_b, self.pw_w, self.pw_b, strict=True):
            t, f = x.shape[2], x.shape[3]
            t1, f1 = self._out(torch.tensor(t)), self._out(torch.tensor(f))
            xp = F.pad(x, (1, 1, 1, 1))
            y = dw_b[None, :, None, None]
            for di in range(3):
                for dj in range(3):
                    y = y + _strided_taps(xp, t1, f1, di, dj) * dw_w[None, :, di, dj, None, None]
            # 1x1 conv: a linear map over the channel axis.
            y = F.linear(y.permute(0, 2, 3, 1), pw_w, pw_b).permute(0, 3, 1, 2)
            x = torch.relu(y)
        b, c, t, f = x.shape
        return self.linear(x.transpose(1, 2).reshape(b, t, c * f))


def rewrite(model, max_frames: int = 512) -> None:
    """Replace the encoder's convolutions in place (eval mode only), for clips up to
    `max_frames` encoder frames of 80 ms."""
    enc = model.encoder
    enc.subsampling = SubsamplingNoConv(enc.subsampling)
    for layer in enc.layers:
        layer.conv = ConvModuleFixedConv(layer.conv, max_frames)
