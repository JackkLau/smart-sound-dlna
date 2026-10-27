import React from 'react';
import { createRoot } from 'react-dom/client';
import MobileApp from './MobileApp.jsx';
import '../../frontend/style.css';
import '../mobile.css';

createRoot(document.getElementById('root')).render(<MobileApp />);
