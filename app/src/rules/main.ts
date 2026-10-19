import '../lib/tokens.css';
import '../lib/contextmenu';
import '../lib/ui.css';
import { mount } from 'svelte';
import Rules from './Rules.svelte';

mount(Rules, { target: document.getElementById('app')! });
