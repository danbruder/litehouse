import { useEffect, useRef, useState } from "react";
import { Button, type ButtonProps } from "./Button";

// Inline two-step confirmation instead of a modal: clicking once swaps the
// button for "<label>? yes / no", which matches the rest of the admin UI's
// flat, paper-not-chrome language and keeps destructive actions (stop,
// remove domain, clear health check) one deliberate step away from a stray
// click. Resets itself after a few seconds so a half-confirmed button never
// lingers.
export function ConfirmButton({
  onConfirm,
  confirmLabel = "sure",
  children,
  ...props
}: ButtonProps & { onConfirm: () => void; confirmLabel?: string }) {
  const [armed, setArmed] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!armed) return;
    timer.current = window.setTimeout(() => setArmed(false), 5_000);
    return () => window.clearTimeout(timer.current);
  }, [armed]);

  if (!armed) {
    return (
      <Button {...props} onClick={() => setArmed(true)}>
        {children}
      </Button>
    );
  }

  return (
    <span className="inline-flex items-center gap-1 text-[0.65rem] uppercase tracking-wide text-ink-2">
      {confirmLabel}?
      <Button
        {...props}
        variant="solid"
        onClick={() => {
          setArmed(false);
          onConfirm();
        }}
      >
        yes
      </Button>
      <Button variant="ghost" onClick={() => setArmed(false)}>
        no
      </Button>
    </span>
  );
}
