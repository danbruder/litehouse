import { AlertTriangle, RotateCw } from "lucide-react";
import { Button } from "./Button";

// Before this, a failed query rendered as an empty card or a perpetual
// "loading…" — indistinguishable from "this app genuinely has no deploys".
// Every page now says what broke and offers a retry.
export function ErrorNote({
  error,
  what,
  onRetry,
}: {
  error: unknown;
  what: string;
  onRetry?: () => void;
}) {
  const message = error instanceof Error ? error.message : String(error);
  return (
    <p className="my-3 flex flex-wrap items-center gap-2 text-xs text-bad">
      <AlertTriangle size={13} />
      <span>
        Couldn't load {what}: <span className="text-ink-2">{message}</span>
      </span>
      {onRetry && (
        <Button variant="ghost" onClick={onRetry}>
          <RotateCw size={11} /> retry
        </Button>
      )}
    </p>
  );
}
