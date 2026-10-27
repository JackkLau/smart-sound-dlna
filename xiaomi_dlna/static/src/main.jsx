import React from 'react';
import { createRoot } from 'react-dom/client';
import LegacyApp from './App.jsx';
import '../style.css';

createRoot(document.getElementById('root')).render(<LegacyApp />);
