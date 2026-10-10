import '../lib/tokens.css';
import { mount } from 'svelte';
import Bar from './Bar.svelte';

mount(Bar, { target: document.getElementById('app')! });

if (import.meta.env.DEV && new URLSearchParams(location.search).has('demo')) void import('./demo');
