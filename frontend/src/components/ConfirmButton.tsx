import { useEffect, useRef, useState } from "react";
import { Button, type ButtonProps } from "./Button";

// Inline two-step confirmation instead of a modal: the first tap swaps the
// button for "<label>? [yes] [no]", keeping destructive actions (stop,
// remove domain, revoke) one deliberate step from a stray tap. Resets after
// a few seconds so a half-confirmed button never lingers.
export function ConfirmButton({
  onConfirm,
  confirmLabel = "sure",
  children,
  size,
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
      <Button {...props} size={size} onClick={() => setArmed(true)}>
        {children}
      </Button>
    );
  }

  return (
    <span className="inline-flex flex-wrap items-center gap-1.5 text-[0.7rem] font-semibold uppercase tracking-wide text-ink-2">
      <span className="mr-0.5">{confirmLabel}?</span>
      <Button
        size={size}
        variant="danger"
        disabled={props.disabled}
        onClick={() => {
          setArmed(false);
          onConfirm();
        }}
      >
        yes
      </Button>
      <Button size={size} variant="ghost" onClick={() => setArmed(false)}>
        no
      </Button>
    </span>
  );
}
