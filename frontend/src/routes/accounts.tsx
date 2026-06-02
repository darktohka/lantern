import {
  CheckCircle2,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Save,
  Trash2,
  X,
} from "lucide-react";
import { FormEvent, useEffect, useMemo, useState } from "react";

import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { Select } from "../components/ui/select";
import { apiFetch } from "../lib/api";
import { useAuth } from "../lib/auth";
import { formatDateTime } from "../lib/utils";

type Service = "hoyoverse" | "ncore";

type HoyoverseConfig = {
  ltoken_v2: string;
  ltuid_v2: string;
  ltmid_v2: string;
};

type NcoreConfig = {
  username: string;
  password: string;
};

type AccountConfig = HoyoverseConfig | NcoreConfig;

type Task = {
  id: number;
  account_id: number;
  account_name: string;
  task_type: string;
  enabled: boolean;
  next_run_at: string;
  last_run_at: string | null;
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

type Account = {
  id: number;
  name: string;
  service: Service;
  enabled: boolean;
  config: AccountConfig;
  created_at: string;
  updated_at: string;
  tasks: Task[];
};

type FormState = {
  editingId: number | null;
  name: string;
  service: Service;
  enabled: boolean;
  config: AccountConfig;
};

const emptyHoyoverseConfig: HoyoverseConfig = {
  ltoken_v2: "",
  ltuid_v2: "",
  ltmid_v2: "",
};

const emptyNcoreConfig: NcoreConfig = {
  username: "",
  password: "",
};

const defaultForm: FormState = {
  editingId: null,
  name: "",
  service: "hoyoverse",
  enabled: true,
  config: emptyHoyoverseConfig,
};

export function AccountsPage() {
  const { token } = useAuth();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [form, setForm] = useState<FormState>(defaultForm);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [runningTasks, setRunningTasks] = useState<Set<number>>(new Set());

  const activeTaskCount = useMemo(
    () => accounts.reduce((total, account) => total + account.tasks.length, 0),
    [accounts],
  );

  async function loadAccounts() {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      setAccounts(await apiFetch<Account[]>("/api/accounts", token));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load accounts");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadAccounts();
  }, [token]);

  function resetForm() {
    setForm({
      ...defaultForm,
      config: { ...emptyHoyoverseConfig },
    });
  }

  function editAccount(account: Account) {
    setForm({
      editingId: account.id,
      name: account.name,
      service: account.service,
      enabled: account.enabled,
      config:
        account.service === "hoyoverse"
          ? ({ ...emptyHoyoverseConfig, ...account.config } as HoyoverseConfig)
          : ({ ...emptyNcoreConfig, ...account.config } as NcoreConfig),
    });
  }

  function setService(service: Service) {
    setForm((current) => ({
      ...current,
      service,
      config:
        service === "hoyoverse"
          ? { ...emptyHoyoverseConfig }
          : { ...emptyNcoreConfig },
    }));
  }

  function setConfigField(field: string, value: string) {
    setForm((current) => ({
      ...current,
      config: {
        ...current.config,
        [field]: value,
      },
    }));
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    if (!token) return;

    setSaving(true);
    setError(null);
    const path = form.editingId
      ? `/api/accounts/${form.editingId}`
      : "/api/accounts";
    const method = form.editingId ? "PUT" : "POST";

    try {
      await apiFetch<Account>(path, token, {
        method,
        body: JSON.stringify({
          name: form.name,
          service: form.service,
          enabled: form.enabled,
          config: form.config,
        }),
      });
      resetForm();
      await loadAccounts();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save account");
    } finally {
      setSaving(false);
    }
  }

  async function runTask(task: Task) {
    if (!token) return;
    setRunningTasks((prev) => new Set(prev).add(task.id));
    setError(null);
    try {
      await apiFetch<TaskLog>(`/api/tasks/${task.id}/run`, token, {
        method: "POST",
      });
      await loadAccounts();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to run task");
    } finally {
      setRunningTasks((prev) => {
        const next = new Set(prev);
        next.delete(task.id);
        return next;
      });
    }
  }

  async function deleteAccount(account: Account) {
    if (!token) return;
    if (!window.confirm(`Delete ${account.name}?`)) return;

    setError(null);
    try {
      await apiFetch<void>(`/api/accounts/${account.id}`, token, {
        method: "DELETE",
      });
      if (form.editingId === account.id) resetForm();
      await loadAccounts();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete account");
    }
  }

  return (
    <main className="mx-auto max-w-6xl px-4 py-6">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">Accounts</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {accounts.length} configured, {activeTaskCount} scheduled tasks
          </p>
        </div>
        <Button variant="outline" onClick={() => void loadAccounts()}>
          <RefreshCw className="h-4 w-4" />
          Refresh
        </Button>
      </div>

      {error ? (
        <div className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      <div className="grid gap-5 lg:grid-cols-[360px_1fr]">
        <Card>
          <CardHeader>
            <CardTitle>{form.editingId ? "Edit account" : "Add account"}</CardTitle>
            <CardDescription>
              Hoyoverse: check-in at 00:00 UTC+8. nCore: check-in between 10:00-14:00 Hungarian time.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="space-y-4" onSubmit={submitForm}>
              <div className="space-y-2">
                <Label htmlFor="account-name">Name</Label>
                <Input
                  id="account-name"
                  value={form.name}
                  onChange={(event) =>
                    setForm((current) => ({
                      ...current,
                      name: event.target.value,
                    }))
                  }
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="service">Service</Label>
                <Select
                  id="service"
                  value={form.service}
                  onChange={(event) => setService(event.target.value as Service)}
                >
                  <option value="hoyoverse">Hoyoverse</option>
                  <option value="ncore">nCore</option>
                </Select>
              </div>

              {form.service === "hoyoverse" ? (
                <HoyoverseFields
                  config={form.config as HoyoverseConfig}
                  onChange={setConfigField}
                />
              ) : (
                <NcoreFields
                  config={form.config as NcoreConfig}
                  onChange={setConfigField}
                />
              )}

              <label className="flex items-center gap-2 text-sm font-medium">
                <input
                  type="checkbox"
                  className="h-4 w-4 rounded border-input accent-primary"
                  checked={form.enabled}
                  onChange={(event) =>
                    setForm((current) => ({
                      ...current,
                      enabled: event.target.checked,
                    }))
                  }
                />
                Enabled
              </label>

              <div className="flex flex-wrap gap-2">
                <Button disabled={saving}>
                  {form.editingId ? (
                    <Save className="h-4 w-4" />
                  ) : (
                    <Plus className="h-4 w-4" />
                  )}
                  {saving ? "Saving..." : form.editingId ? "Save" : "Add"}
                </Button>
                {form.editingId ? (
                  <Button type="button" variant="secondary" onClick={resetForm}>
                    <X className="h-4 w-4" />
                    Cancel
                  </Button>
                ) : null}
              </div>
            </form>
          </CardContent>
        </Card>

        <div className="space-y-3">
          {loading ? (
            <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
              Loading accounts...
            </div>
          ) : accounts.length === 0 ? (
            <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
              No accounts configured.
            </div>
          ) : (
            accounts.map((account) => (
              <Card key={account.id}>
                <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <CardTitle className="truncate">{account.name}</CardTitle>
                      <Badge variant="secondary">
                        {account.service === "hoyoverse" ? "Hoyoverse" : "nCore"}
                      </Badge>
                      <Badge variant={account.enabled ? "success" : "secondary"}>
                        {account.enabled ? "Enabled" : "Disabled"}
                      </Badge>
                    </div>
                    <CardDescription>
                      Updated {formatDateTime(account.updated_at)}
                    </CardDescription>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => editAccount(account)}
                    >
                      <Pencil className="h-4 w-4" />
                      <span className="sr-only">Edit</span>
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => void deleteAccount(account)}
                    >
                      <Trash2 className="h-4 w-4" />
                      <span className="sr-only">Delete</span>
                    </Button>
                  </div>
                </CardHeader>
                <CardContent className="space-y-3">
                  {account.tasks.length > 0 ? (
                    account.tasks.map((task) => (
                      <div
                        key={task.id}
                        className="grid gap-2 rounded-md border border-border px-3 py-3 text-sm sm:grid-cols-[1fr_auto]"
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2 font-medium">
                            <CheckCircle2 className="h-4 w-4 text-primary" />
                            {task.task_type === "ncore_daily_checkin"
                              ? "nCore Daily Check-in"
                              : task.task_type === "ncore_hitnrun_check"
                                ? "nCore Torrent Refresh"
                                : "Hoyoverse Daily Check-in"}
                          </div>
                          <div className="mt-1 text-muted-foreground">
                            Last run: {formatDateTime(task.last_run_at)}
                          </div>
                        </div>
                        <div className="flex flex-col items-end gap-2 text-muted-foreground">
                          <div>Next run: {formatDateTime(task.next_run_at)}</div>
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={runningTasks.has(task.id)}
                            onClick={() => void runTask(task)}
                          >
                            <Play className="h-3 w-3" />
                            {runningTasks.has(task.id) ? "Running..." : "Run"}
                          </Button>
                        </div>
                      </div>
                    ))
                  ) : (
                    <div className="text-sm text-muted-foreground">
                      No scheduled tasks.
                    </div>
                  )}
                </CardContent>
              </Card>
            ))
          )}
        </div>
      </div>
    </main>
  );
}

function HoyoverseFields({
  config,
  onChange,
}: {
  config: HoyoverseConfig;
  onChange: (field: keyof HoyoverseConfig, value: string) => void;
}) {
  return (
    <>
      <div className="space-y-2">
        <Label htmlFor="ltoken-v2">ltoken_v2</Label>
        <Input
          id="ltoken-v2"
          type="password"
          value={config.ltoken_v2}
          onChange={(event) => onChange("ltoken_v2", event.target.value)}
          required
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="ltuid-v2">ltuid_v2</Label>
        <Input
          id="ltuid-v2"
          value={config.ltuid_v2}
          onChange={(event) => onChange("ltuid_v2", event.target.value)}
          required
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="ltmid-v2">ltmid_v2</Label>
        <Input
          id="ltmid-v2"
          type="password"
          value={config.ltmid_v2}
          onChange={(event) => onChange("ltmid_v2", event.target.value)}
          required
        />
      </div>
    </>
  );
}

function NcoreFields({
  config,
  onChange,
}: {
  config: NcoreConfig;
  onChange: (field: keyof NcoreConfig, value: string) => void;
}) {
  return (
    <>
      <div className="space-y-2">
        <Label htmlFor="ncore-username">Username</Label>
        <Input
          id="ncore-username"
          value={config.username}
          autoComplete="username"
          onChange={(event) => onChange("username", event.target.value)}
          required
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="ncore-password">Password</Label>
        <Input
          id="ncore-password"
          type="password"
          value={config.password}
          autoComplete="new-password"
          onChange={(event) => onChange("password", event.target.value)}
          required
        />
      </div>
    </>
  );
}
