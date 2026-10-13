// Dev only (npm run dev, then promo.html): the README's demo, recorded by tools/media/record.py.
// Not in the production build: vite.config.ts lists the app's windows only.
import '../lib/tokens.css';
import { mount } from 'svelte';
import Promo from './Promo.svelte';

mount(Promo, { target: document.getElementById('app')! });
