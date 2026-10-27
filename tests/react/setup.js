import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';
afterEach(() => {
  cleanup();
  sessionStorage.clear();
  localStorage.clear();
});
// jsdom has no native modal implementation. Browser smoke tests also exercise
// these components with the actual dialog API, keyboard and focus behavior.
HTMLDialogElement.prototype.showModal = function () {
  this.open = true;
  this.querySelector('[autofocus], input, button')?.focus();
};
HTMLDialogElement.prototype.close = function () {
  this.open = false;
  this.dispatchEvent(new Event('close'));
};
HTMLElement.prototype.scrollIntoView = function () {};
