import '../lib/tokens.css';
import '../lib/ui.css';
import { mount } from 'svelte';
import Home from './Home.svelte';

mount(Home, { target: document.getElementById('app')! });
