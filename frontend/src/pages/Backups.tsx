import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { relativeTime, formatBytes } from "../lib/format";

export function Backups() {
  const { data: backups, isLoading } = useQuery({
    queryKey: ["backups-catalog"],
    queryFn: api.backupCatalog,
    refetchInterval: 30_000,
  });

  return (
    <>
      <p>
        <Link to="/">&larr; all apps</Link>
      </p>

      <h2>Backups</h2>

      {isLoading ? (
        <p className="muted">loading…</p>
      ) : !backups || backups.length === 0 ? (
        <div className="card">
          <span className="panel-label">catalog</span>
          <p className="muted">No backups recorded yet — the catalog fills in as backups run.</p>
        </div>
      ) : (
        <table>
          <thead>
            <tr>
              <th>App</th>
              <th>Date</th>
              <th>Size</th>
              <th>Age</th>
            </tr>
          </thead>
          <tbody>
            {backups.map((b) => (
              <tr key={b.id}>
                <td>{b.app_name}</td>
                <td>{b.s3_key}</td>
                <td>{formatBytes(b.size_bytes)}</td>
                <td>{relativeTime(b.created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}
