import { setNonce } from 'get-nonce';

// Tauri attaches a per-document nonce to the startup stylesheet. Reuse it for
// Radix's generated styles and react-remove-scroll's measured scrollbar rules.
export const styleNonce = document.querySelector<HTMLStyleElement>('style[nonce]')?.nonce || undefined;
if (styleNonce) setNonce(styleNonce);
