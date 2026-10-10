import '../lib/tokens.css';
import { mount } from 'svelte';
import Bar from './Bar.svelte';

mount(Bar, { target: document.getElementById('app')! });
