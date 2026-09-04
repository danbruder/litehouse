import { type ButtonHTMLAttributes, forwardRef } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../lib/cn";

// Deliberately thin: reuses the existing .btn-outline look from
// styles.css (transparent, lime fill on hover, hairline border via the
// button's own default browser border — no radius, no shadow, matching the
// rest of the admin UI's "paper, not chrome" system) rather than
// introducing a second button language.
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 border font-medium uppercase tracking-wide transition-colors disabled:opacity-40 disabled:pointer-events-none",
  {
    variants: {
      variant: {
        outline: "btn-outline border-ink text-[0.65rem] px-3 py-1.5",
        ghost: "border-transparent text-ink-2 hover:text-ink text-[0.65rem] px-2 py-1",
        solid: "bg-lime border-lime text-on-lime hover:opacity-90 text-[0.65rem] px-3 py-1.5",
      },
      size: {
        sm: "text-[0.65rem] px-2.5 py-1",
        md: "text-xs px-3 py-1.5",
      },
    },
    defaultVariants: { variant: "outline", size: "sm" },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, ...props }, ref) => (
    <button ref={ref} className={cn(buttonVariants({ variant, size }), className)} {...props} />
  ),
);
Button.displayName = "Button";
