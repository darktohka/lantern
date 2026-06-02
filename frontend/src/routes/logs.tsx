import { ChevronLeft, ChevronRight, RefreshCw, ScrollText } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Select } from "../components/ui/select";
import { apiFetch } from "../lib/api";
import { useAuth } from "../lib/auth";
import { formatDateTime, formatDuration } from "../lib/utils";

type Account = {
  id: number;
  name: string;
};

type TaskLog = {
  id: number;
  account_id: number | null;
  account_name: string | null;
  task_type: string;
  status: string;
  started_at: string;
  finished_at: string;
  duration_ms: number;
  message: string;
};

type LogsResponse = {
  page: number;
  page_size: number;
  total: number;
  items: TaskLog[];
};

const pageSize = 20;

export function LogsPage() {
  const { token } = useAuth();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [logs, setLogs] = useState<LogsResponse>({
    page: 1,
    page_size: pageSize,
    total: 0,
    items: [],
  });
  const [page, setPage] = useState(1);
  const [accountId, setAccountId] = useState<string>("all");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const totalPages = useMemo(
    () => Math.max(1, Math.ceil(logs.total / logs.page_size)),
    [logs.total, logs.page_size],
  );

  async function loadLogs(nextPage = page) {
    if (!token) return;
    setLoading(true);
    setError(null);
    const params = new URLSearchParams({
      page: String(nextPage),
      page_size: String(pageSize),
    });
    if (accountId !== "all") params.set("account_id", accountId);

    try {
      const [accountResponse, logResponse] = await Promise.all([
        apiFetch<Account[]>("/api/accounts", token),
        apiFetch<LogsResponse>(`/api/task-logs?${params}`, token),
      ]);
      setAccounts(accountResponse);
      setLogs(logResponse);
      setPage(nextPage);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load logs");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadLogs(1);
  }, [token, accountId]);

  return (
    <main className="mx-auto max-w-6xl px-4 py-6">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">Execution logs</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {logs.total} recorded task runs
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Select
            className="w-56"
            value={accountId}
            onChange={(event) => setAccountId(event.target.value)}
          >
            <option value="all">All accounts</option>
            {accounts.map((account) => (
              <option key={account.id} value={account.id}>
                {account.name}
              </option>
            ))}
          </Select>
          <Button variant="outline" onClick={() => void loadLogs(page)}>
            <RefreshCw className="h-4 w-4" />
            Refresh
          </Button>
        </div>
      </div>

      {error ? (
        <div className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <ScrollText className="h-5 w-5 text-primary" />
            <CardTitle>Task runs</CardTitle>
          </div>
          <CardDescription>Timestamp and execution time for each run.</CardDescription>
        </CardHeader>
        <CardContent>
          {loading ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              Loading logs...
            </div>
          ) : logs.items.length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              No logs found.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[860px] border-collapse text-sm">
                <thead>
                  <tr className="border-b border-border text-left text-muted-foreground">
                    <th className="py-3 pr-4 font-medium">Started</th>
                    <th className="py-3 pr-4 font-medium">Account</th>
                    <th className="py-3 pr-4 font-medium">Task</th>
                    <th className="py-3 pr-4 font-medium">Status</th>
                    <th className="py-3 pr-4 font-medium">Duration</th>
                    <th className="py-3 font-medium">Message</th>
                  </tr>
                </thead>
                <tbody>
                  {logs.items.map((log) => (
                    <tr key={log.id} className="border-b border-border/70 align-top">
                      <td className="py-3 pr-4 text-muted-foreground">
                        {formatDateTime(log.started_at)}
                      </td>
                      <td className="py-3 pr-4">{log.account_name ?? "-"}</td>
                      <td className="py-3 pr-4">
                        {log.task_type === "ncore_daily_checkin"
                          ? "nCore Daily Check-in"
                          : log.task_type === "ncore_hitnrun_check"
                            ? "nCore Torrent Refresh"
                            : "Hoyoverse Daily Check-in"}
                      </td>
                      <td className="py-3 pr-4">
                        <Badge
                          variant={log.status === "success" ? "success" : "destructive"}
                        >
                          {log.status}
                        </Badge>
                      </td>
                      <td className="py-3 pr-4 text-muted-foreground">
                        {formatDuration(log.duration_ms)}
                      </td>
                      <td className="max-w-md py-3 text-muted-foreground">
                        {log.message}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          <div className="mt-4 flex items-center justify-between gap-3">
            <div className="text-sm text-muted-foreground">
              Page {logs.page} of {totalPages}
            </div>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => void loadLogs(page - 1)}
              >
                <ChevronLeft className="h-4 w-4" />
                Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => void loadLogs(page + 1)}
              >
                Next
                <ChevronRight className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>
    </main>
  );
}
