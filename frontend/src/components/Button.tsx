import { type ButtonHTMLAttributes, forwardRef } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../lib/cn";

// Same "paper, not chrome" language as styles.css (hairlines, no radius,
// lime on hover), sized for thumbs: `md` is a 44px target on phones and
// shrinks to 36px once there's a pointer. styles.css gives every bare
// <button> a solid ink fill, so each variant sets its own background.
const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap border font-semibold uppercase tracking-wide transition-colors disabled:pointer-events-none disabled:opacity-40",
  {
    variants: {
      variant: {
        outline: "btn-outline border-ink bg-transparent text-ink",
        ghost:
          "border-transparent bg-transparent text-ink-2 hover:border-transparent hover:bg-paper-3 hover:text-ink",
        solid: "border-ink bg-ink text-paper hover:border-lime hover:bg-lime hover:text-on-lime",
        lime: "border-lime bg-lime text-on-lime hover:opacity-90",
        danger:
          "border-bad bg-transparent text-bad hover:border-bad hover:bg-bad hover:text-paper",
      },
      size: {
        sm: "min-h-[32px] px-2.5 text-[0.65rem]",
        md: "min-h-[44px] px-3.5 text-[0.7rem] md:min-h-[36px]",
        icon: "h-11 w-11 p-0 md:h-9 md:w-9",
      },
    },
    defaultVariants: { variant: "outline", size: "sm" },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, type = "button", ...props }, ref) => (
    <button ref={ref} type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />
  ),
);
Button.displayName = "Button";
