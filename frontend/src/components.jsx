import { Children, createContext, useContext, useLayoutEffect, useRef, useState } from 'react';
import * as SelectPrimitive from '@radix-ui/react-select';
import { setNonce } from 'get-nonce';

// Tauri gives this bundled style element a fresh CSP nonce for each page load.
// Share it with Radix's viewport and scroll-lock stylesheet, without allowing
// arbitrary inline styles in the application's security policy.
const styleNonce = document.getElementById('ui-style-nonce')?.nonce || undefined;
if (styleNonce) setNonce(styleNonce);

const SelectContainer = createContext(null);

// A single accessible popup recipe. Modal content must stay in the native
// dialog's top layer, otherwise the browser makes its options inert.
export function Select({ children, value, onChange, disabled, className = '', ...props }) {
  const container = useContext(SelectContainer);
  const options = Children.toArray(children);
  return (
    <SelectPrimitive.Root
      value={String(value ?? '')}
      disabled={disabled}
      onValueChange={(next) => onChange?.({ target: { value: next } })}
    >
      <SelectPrimitive.Trigger {...props} className={`select-trigger ${className}`}>
        <SelectPrimitive.Value placeholder="请选择" />
        <SelectPrimitive.Icon>
          <Icon name="arrow" className="select-chevron" />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal container={container || undefined}>
        <SelectPrimitive.Content
          className="select-popup"
          position="popper"
          sideOffset={6}
          collisionPadding={12}
          onEscapeKeyDown={(event) => event.stopPropagation()}
        >
          <SelectPrimitive.ScrollUpButton className="select-scroll">
            ⌃
          </SelectPrimitive.ScrollUpButton>
          <SelectPrimitive.Viewport nonce={styleNonce}>
            {options.map((option) => (
              <SelectPrimitive.Item
                className="select-option"
                key={option.props.value}
                value={String(option.props.value || '__empty__')}
                disabled={option.props.disabled || !option.props.value}
              >
                <SelectPrimitive.ItemText>{option.props.children}</SelectPrimitive.ItemText>
                <SelectPrimitive.ItemIndicator>
                  <Icon name="check" />
                </SelectPrimitive.ItemIndicator>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
          <SelectPrimitive.ScrollDownButton className="select-scroll">
            ⌄
          </SelectPrimitive.ScrollDownButton>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}

export function ClearableInput({
  inputRef,
  value,
  onClear,
  disabled,
  clearLabel = '清除链接',
  ...props
}) {
  return (
    <div className="clearable-field">
      <input {...props} ref={inputRef} value={value} disabled={disabled} />
      {value && (
        <button
          type="button"
          className="field-clear"
          aria-label={clearLabel}
          disabled={disabled}
          onClick={() => {
            onClear();
            inputRef.current?.focus();
          }}
        >
          <Icon name="close" />
        </button>
      )}
    </div>
  );
}

const icons = {
  music: (
    <>
      <path d="M9 18V5l12-2v13M9 9l12-2" />
      <ellipse cx="6" cy="18" rx="3" ry="2" />
      <ellipse cx="18" cy="16" rx="3" ry="2" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  search: (
    <>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <path d="m16 16 4 4" />
    </>
  ),
  close: <path d="m6 6 12 12M18 6 6 18" />,
  play: <path d="m8 5 11 7-11 7Z" />,
  pause: <path d="M8 5v14M16 5v14" />,
  stop: <rect x="6" y="6" width="12" height="12" rx="1" />,
  previous: <path d="M5 5v14m14-14-10 7 10 7Z" />,
  next: <path d="M19 5v14M5 5l10 7-10 7Z" />,
  volume: <path d="m11 4-6 5H2v6h3l6 5ZM15 8a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14" />,
  speaker: (
    <>
      <rect x="5" y="2" width="14" height="20" rx="3" />
      <circle cx="12" cy="14" r="4" />
      <path d="M11 6h2" />
    </>
  ),
  refresh: (
    <>
      <path d="M20 7v5h-5M4 17v-5h5" />
      <path d="M6 7a7 7 0 0 1 12-1l2 3M4 15l2 3a7 7 0 0 0 12-1" />
    </>
  ),
  folder: <path d="M3 7V5a2 2 0 0 1 2-2h5l3 4h6a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />,
  arrow: <path d="m9 5 7 7-7 7" />,
  check: <path d="m5 12 4 4L19 6" />,
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v6M12 7h.01" />
    </>
  ),
  upload: (
    <>
      <path d="M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5" />
    </>
  ),
};

export function Icon({ name, className = '' }) {
  return (
    <svg className={`icon ${className}`.trim()} viewBox="0 0 24 24" aria-hidden="true">
      {icons[name] || icons.music}
    </svg>
  );
}

// Imperative access is limited to the platform dialog/focus API. All UI and
// pending state remain owned by React; no HTML or controls are constructed here.
export function Dialog({
  id,
  title,
  eyebrow,
  description,
  open,
  onClose,
  busy = false,
  children,
  className = 'feature-dialog',
}) {
  const ref = useRef(null);
  const [container, setContainer] = useState(null);
  useLayoutEffect(() => {
    const dialog = ref.current;
    setContainer(dialog);
    if (!open) {
      if (dialog.open) dialog.close();
      return;
    }
    const opener = document.activeElement;
    if (!dialog.open) dialog.showModal();
    dialog.querySelector('[data-autofocus], [autofocus]')?.focus();
    return () => {
      if (dialog.open) dialog.close();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, [open]);
  return (
    <dialog
      ref={ref}
      id={id}
      className={className}
      aria-labelledby={`${id}-title`}
      aria-describedby={description}
      aria-busy={busy}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      <div className="dialog-heading">
        <div>
          {eyebrow && <span className="eyebrow">{eyebrow}</span>}
          <h2 id={`${id}-title`}>{title}</h2>
        </div>
        <button
          type="button"
          className="icon-button dialog-close"
          disabled={busy}
          onClick={onClose}
          aria-label={`关闭${title}`}
          title="关闭"
        >
          <Icon name="close" />
        </button>
      </div>
      <SelectContainer.Provider value={container}>{children}</SelectContainer.Provider>
    </dialog>
  );
}
