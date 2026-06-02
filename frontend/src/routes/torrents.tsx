import { RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { apiFetch } from "../lib/api";
import { useAuth } from "../lib/auth";
import { formatDateTime } from "../lib/utils";

type Torrent = {
  id: number;
  account_id: number;
  account_name: string;
  ncore_id: string;
  info_hash: string | null;
  name: string;
  status: string;
  hnr_timespent: string | null;
  hnr_seed: string | null;
  progress: number;
  download_rate: number;
  upload_rate: number;
  total_download: number;
  total_upload: number;
  created_at: string;
  updated_at: string;
};

type GroupedTorrents = Record<string, Torrent[]>;

const statusBadge: Record<string, { label: string; variant: "default" | "secondary" | "success" | "destructive" }> = {
  pending: { label: "Pending", variant: "secondary" },
  downloading: { label: "Downloading", variant: "default" },
  seeding: { label: "Seeding", variant: "success" },
  complete: { label: "Complete", variant: "success" },
  paused: { label: "Paused", variant: "secondary" },
  failed: { label: "Failed", variant: "destructive" },
};

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / 1024 ** i).toFixed(1)} ${units[i]}`;
}

function formatRate(bytesPerSec: number): string {
  if (bytesPerSec === 0) return "-";
  return `${formatBytes(bytesPerSec)}/s`;
}

export function TorrentsPage() {
  const { token } = useAuth();
  const [torrents, setTorrents] = useState<Torrent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  async function loadTorrents() {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      setTorrents(await apiFetch<Torrent[]>("/api/torrents", token));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load torrents");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadTorrents();
  }, [token]);

  async function removeTorrent(torrent: Torrent) {
    if (!token) return;
    if (!window.confirm(`Remove "${torrent.name}"? This will delete the files.`)) return;

    setError(null);
    try {
      await apiFetch<void>(`/api/torrents/${torrent.id}`, token, {
        method: "DELETE",
      });
      await loadTorrents();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to remove torrent");
    }
  }

  const grouped = torrents.reduce<GroupedTorrents>((acc, t) => {
    const key = t.account_name || "Unknown";
    (acc[key] ??= []).push(t);
    return acc;
  }, {});

  return (
    <main className="mx-auto max-w-6xl px-4 py-6">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">Torrents</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {torrents.length} tracked torrent{torrents.length !== 1 ? "s" : ""}
          </p>
        </div>
        <Button variant="outline" onClick={() => void loadTorrents()}>
          <RefreshCw className="h-4 w-4" />
          Refresh
        </Button>
      </div>

      {error ? (
        <div className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      {loading ? (
        <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
          Loading torrents...
        </div>
      ) : Object.keys(grouped).length === 0 ? (
        <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
          No torrents tracked. Torrents will appear here when nCore accounts have stopped torrents that need seeding.
        </div>
      ) : (
        Object.entries(grouped).map(([accountName, accountTorrents]) => (
          <Card key={accountName} className="mb-4">
            <CardHeader>
              <CardTitle>{accountName}</CardTitle>
              <CardDescription>
                {accountTorrents.length} torrent{accountTorrents.length !== 1 ? "s" : ""}
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-border text-left text-muted-foreground">
                      <th className="pb-2 pr-4 font-medium">Name</th>
                      <th className="pb-2 pr-4 font-medium">Status</th>
                      <th className="pb-2 pr-4 font-medium">Progress</th>
                      <th className="pb-2 pr-4 font-medium">DL</th>
                      <th className="pb-2 pr-4 font-medium">UL</th>
                      <th className="pb-2 pr-4 font-medium">Remaining</th>
                      <th className="pb-2 pr-4 font-medium">Updated</th>
                      <th className="pb-2 font-medium" />
                    </tr>
                  </thead>
                  <tbody>
                    {accountTorrents.map((t) => {
                      const badge = statusBadge[t.status] ?? { label: t.status, variant: "secondary" };
                      return (
                        <tr key={t.id} className="border-b border-border/50 last:border-0">
                          <td className="py-2 pr-4">
                            <div className="max-w-64 truncate font-medium" title={t.name}>
                              {t.name || `#${t.ncore_id}`}
                            </div>
                          </td>
                          <td className="py-2 pr-4">
                            <Badge variant={badge.variant}>{badge.label}</Badge>
                          </td>
                          <td className="py-2 pr-4">
                            {t.status === "downloading" || t.status === "seeding"
                              ? `${(t.progress * 100).toFixed(1)}%`
                              : "-"}
                          </td>
                          <td className="py-2 pr-4 text-muted-foreground">
                            {formatRate(t.download_rate)}
                          </td>
                          <td className="py-2 pr-4 text-muted-foreground">
                            {formatRate(t.upload_rate)}
                          </td>
                          <td className="py-2 pr-4 text-muted-foreground">
                            {t.hnr_timespent || "-"}
                          </td>
                          <td className="py-2 pr-4 text-muted-foreground">
                            {formatDateTime(t.updated_at)}
                          </td>
                          <td className="py-2">
                            <Button
                              variant="ghost"
                              size="icon"
                              onClick={() => void removeTorrent(t)}
                            >
                              <Trash2 className="h-4 w-4" />
                              <span className="sr-only">Remove</span>
                            </Button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </CardContent>
          </Card>
        ))
      )}
    </main>
  );
}
