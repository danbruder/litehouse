import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api } from "./api";

const PAST = { start: "started", stop: "stopped", restart: "restarted" } as const;

/** Start/stop/restart one app, with a toast and the list + detail queries
 *  refreshed afterwards. Shared by the dashboard rows and the app page. */
export function useAppAction(action: "start" | "stop" | "restart") {
  const qc = useQueryClient();
  const fn = { start: api.startApp, stop: api.stopApp, restart: api.restartApp }[action];
  return useMutation({
    mutationFn: (name: string) => fn(name),
    onSuccess: (_data, name) => {
      toast.success(`${name} ${PAST[action]}`);
      qc.invalidateQueries({ queryKey: ["app-summary", name] });
      qc.invalidateQueries({ queryKey: ["apps-summary"] });
    },
    onError: (err: Error, name) => toast.error(`Failed to ${action} ${name}`, { description: err.message }),
  });
}

export function useRunBackup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.runBackup,
    onSuccess: (report) => {
      if (report.failed.length > 0) {
        toast.error(`Backup finished with ${report.failed.length} failure(s)`, {
          description: report.failed.map(([name]) => name).join(", "),
        });
      } else {
        toast.success(`Backup finished: ${report.succeeded.length} app(s)`);
      }
      qc.invalidateQueries({ queryKey: ["backup-status"] });
      qc.invalidateQueries({ queryKey: ["backups-catalog"] });
    },
    onError: (err: Error) => toast.error("Backup failed", { description: err.message }),
  });
}
