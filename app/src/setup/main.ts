import '../lib/tokens.css';
import '../lib/ui.css';
import { mount } from 'svelte';
import Setup from './Setup.svelte';

mount(Setup, { target: document.getElementById('app')! });
