import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { Tooltip } from 'radix-ui';

import { App } from './App';
import './styles/global.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root');

createRoot(root).render(
  <StrictMode>
    <Tooltip.Provider delayDuration={300}>
      <App />
    </Tooltip.Provider>
  </StrictMode>,
);
