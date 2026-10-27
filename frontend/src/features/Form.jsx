import React, { useRef } from 'react';

export function Form({ children, onSubmit, ...props }) {
  const composing = useRef(false);
  return (
    <form
      {...props}
      noValidate
      onCompositionStart={() => {
        composing.current = true;
      }}
      onCompositionEnd={() => {
        composing.current = false;
      }}
      onSubmit={(event) => {
        event.preventDefault();
        if (!composing.current && !event.nativeEvent.isComposing) onSubmit?.(event);
      }}
    >
      {children}
    </form>
  );
}

export function ErrorMessage({ id, children }) {
  return (
    <p id={id} className="field-error" role="alert">
      {children || ''}
    </p>
  );
}
