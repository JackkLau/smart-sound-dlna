import { build } from 'vite';
for (const mode of ['desktop', 'mobile', 'legacy']) await build({ mode });
