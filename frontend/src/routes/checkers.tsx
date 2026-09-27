import {
  ChevronLeft,
  ChevronRight,
  FileDiff,
  Globe,
  History,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";

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
import { lineDiff, type DiffLine } from "../lib/diff";
import { formatDateTime, formatDuration } from "../lib/utils";

type CheckType = "content_changed" | "status_code_changed";
type StatusAlertMode = "match" | "mismatch" | "any";

type CheckerConfig = {
  selector?: string | null;
  ignore_whitespace?: boolean;
  target_status?: number | null;
  mode?: StatusAlertMode;
  alert_on_error?: boolean;
};

type Checker = {
  id: number;
  name: string;
  url: string;
  check_type: CheckType;
  interval_seconds: number;
  enabled: boolean;
  config: CheckerConfig;
  next_run_at: string;
  last_run_at: string | null;
  previous_status_code: number | null;
  last_status_code: number | null;
  last_changed_at: string | null;
  has_previous_content: boolean;
  has_current_content: boolean;
  created_at: string;
  updated_at: string;
};

type CreateCheckerRequest = {
  name: string;
  url: string;
  check_type: CheckType;
  interval_seconds: number;
  enabled: boolean;
  config: CheckerConfig;
};

type CheckerResult = {
  id: number;
  checker_id: number;
  status_code: number | null;
  status_changed: boolean;
  content_changed: boolean;
  triggered: boolean;
  message: string;
  started_at: string;
  finished_at: string;
  duration_ms: number;
};

type CheckerDiff = {
  checker_id: number;
  check_type: CheckType;
  previous_content: string | null;
  current_content: string | null;
  last_changed_at: string | null;
  previous_status_code: number | null;
  last_status_code: number | null;
};

type Paginated<T> = {
  page: number;
  page_size: number;
  total: number;
  items: T[];
};

type IntervalUnit = "seconds" | "minutes" | "hours";

type FormState = {
  name: string;
  url: string;
  checkType: CheckType;
  intervalValue: string;
  intervalUnit: IntervalUnit;
  enabled: boolean;
  selector: string;
  ignoreWhitespace: boolean;
  targetStatus: string;
  mode: StatusAlertMode;
  alertOnError: boolean;
};

const RESULTS_PAGE_SIZE = 10;

const UNIT_SECONDS: Record<IntervalUnit, number> = {
  seconds: 1,
  minutes: 60,
  hours: 3600,
};

const EMPTY_FORM: FormState = {
  name: "",
  url: "",
  checkType: "content_changed",
  intervalValue: "60",
  intervalUnit: "seconds",
  enabled: true,
  selector: "",
  ignoreWhitespace: true,
  targetStatus: "",
  mode: "match",
  alertOnError: true,
};

function splitInterval(seconds: number): { value: number; unit: IntervalUnit } {
  if (seconds > 0 && seconds % 3600 === 0) {
    return { value: seconds / 3600, unit: "hours" };
  }
  if (seconds > 0 && seconds % 60 === 0) {
    return { value: seconds / 60, unit: "minutes" };
  }
  return { value: seconds, unit: "seconds" };
}

function formatInterval(seconds: number): string {
  const { value, unit } = splitInterval(seconds);
  const label = value === 1 ? unit.slice(0, -1) : unit;
  return `Every ${value} ${label}`;
}

function checkTypeLabel(checkType: CheckType): string {
  return checkType === "content_changed"
    ? "Content changed"
    : "Status code changed";
}

function formatStatusCode(code: number | null): string {
  return code === null ? "-" : String(code);
}

function diffLineClass(type: DiffLine["type"]): string {
  switch (type) {
    case "added":
      return "bg-emerald-500/15 text-emerald-700";
    case "removed":
      return "bg-destructive/10 text-destructive";
    case "same":
      return "text-muted-foreground";
  }
}

function diffPrefix(type: DiffLine["type"]): string {
  switch (type) {
    case "added":
      return "+";
    case "removed":
      return "-";
    case "same":
      return " ";
  }
}

function buildConfig(form: FormState): CheckerConfig {
  if (form.checkType === "content_changed") {
    const selector = form.selector.trim();
    return {
      selector: selector === "" ? null : selector,
      ignore_whitespace: form.ignoreWhitespace,
      alert_on_error: form.alertOnError,
    };
  }
  const target = form.targetStatus.trim();
  const targetStatus = target === "" ? null : Number(target);
  return {
    target_status:
      targetStatus !== null && Number.isFinite(targetStatus) ? targetStatus : null,
    mode: form.mode,
    alert_on_error: form.alertOnError,
  };
}

function buildRequest(form: FormState): CreateCheckerRequest | null {
  const seconds = Math.round(
    Number(form.intervalValue) * UNIT_SECONDS[form.intervalUnit],
  );
  if (!Number.isFinite(seconds) || seconds < 60) return null;
  return {
    name: form.name.trim(),
    url: form.url.trim(),
    check_type: form.checkType,
    interval_seconds: seconds,
    enabled: form.enabled,
    config: buildConfig(form),
  };
}

function Info({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className="text-xs uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div className="text-sm">{value}</div>
    </div>
  );
}

export function CheckersPage() {
  const { token } = useAuth();
  const [checkers, setCheckers] = useState<Checker[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [busy, setBusy] = useState<Set<number>>(new Set());
  const [diff, setDiff] = useState<{ checkerId: number; data: CheckerDiff } | null>(
    null,
  );
  const [results, setResults] = useState<{
    checkerId: number;
    data: Paginated<CheckerResult>;
  } | null>(null);

  async function loadCheckers() {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      setCheckers(await apiFetch<Checker[]>("/api/checkers", token));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load checkers");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadCheckers();
  }, [token]);

  function setBusyFor(id: number, active: boolean) {
    setBusy((prev) => {
      const next = new Set(prev);
      if (active) next.add(id);
      else next.delete(id);
      return next;
    });
  }

  function resetForm() {
    setEditingId(null);
    setForm(EMPTY_FORM);
  }

  function startEdit(checker: Checker) {
    const { value, unit } = splitInterval(checker.interval_seconds);
    setEditingId(checker.id);
    setForm({
      name: checker.name,
      url: checker.url,
      checkType: checker.check_type,
      intervalValue: String(value),
      intervalUnit: unit,
      enabled: checker.enabled,
      selector: checker.config.selector ?? "",
      ignoreWhitespace: checker.config.ignore_whitespace ?? true,
      targetStatus:
        checker.config.target_status === null ||
        checker.config.target_status === undefined
          ? ""
          : String(checker.config.target_status),
      mode: checker.config.mode ?? "match",
      alertOnError: checker.config.alert_on_error ?? true,
    });
    setError(null);
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    if (!token) return;

    const request = buildRequest(form);
    if (!request) {
      setError("Schedule must be at least 60 seconds.");
      return;
    }

    setSaving(true);
    setError(null);
    try {
      if (editingId === null) {
        await apiFetch<Checker>("/api/checkers", token, {
          method: "POST",
          body: JSON.stringify(request),
        });
      } else {
        await apiFetch<Checker>(`/api/checkers/${editingId}`, token, {
          method: "PUT",
          body: JSON.stringify(request),
        });
      }
      resetForm();
      await loadCheckers();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save checker");
    } finally {
      setSaving(false);
    }
  }

  async function deleteChecker(checker: Checker) {
    if (!token) return;
    if (!window.confirm(`Delete checker "${checker.name}"?`)) return;

    setError(null);
    try {
      await apiFetch<void>(`/api/checkers/${checker.id}`, token, {
        method: "DELETE",
      });
      if (editingId === checker.id) resetForm();
      if (diff?.checkerId === checker.id) setDiff(null);
      if (results?.checkerId === checker.id) setResults(null);
      await loadCheckers();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete checker");
    }
  }

  async function runChecker(checker: Checker) {
    if (!token) return;
    setBusyFor(checker.id, true);
    setError(null);
    try {
      const result = await apiFetch<CheckerResult>(
        `/api/checkers/${checker.id}/run`,
        token,
        { method: "POST" },
      );
      window.alert(result.message);
      await loadCheckers();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to run checker");
    } finally {
      setBusyFor(checker.id, false);
    }
  }

  async function toggleDiff(checker: Checker) {
    if (!token) return;
    if (diff?.checkerId === checker.id) {
      setDiff(null);
      return;
    }

    setBusyFor(checker.id, true);
    setError(null);
    try {
      const data = await apiFetch<CheckerDiff>(
        `/api/checkers/${checker.id}/diff`,
        token,
      );
      setDiff({ checkerId: checker.id, data });
      setResults(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load diff");
    } finally {
      setBusyFor(checker.id, false);
    }
  }

  async function loadResults(checker: Checker, page: number) {
    if (!token) return;
    setBusyFor(checker.id, true);
    setError(null);
    try {
      const data = await apiFetch<Paginated<CheckerResult>>(
        `/api/checkers/${checker.id}/results?page=${page}&page_size=${RESULTS_PAGE_SIZE}`,
        token,
      );
      setResults({ checkerId: checker.id, data });
      setDiff(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load results");
    } finally {
      setBusyFor(checker.id, false);
    }
  }

  function toggleResults(checker: Checker) {
    if (results?.checkerId === checker.id) {
      setResults(null);
      return;
    }
    void loadResults(checker, 1);
  }

  return (
    <main className="mx-auto max-w-6xl px-4 py-6">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">Web checkers</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {checkers.length} checker{checkers.length !== 1 ? "s" : ""} configured
          </p>
        </div>
        <Button variant="outline" onClick={() => void loadCheckers()}>
          <RefreshCw className="h-4 w-4" />
          Refresh
        </Button>
      </div>

      {error ? (
        <div className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : null}

      <div className="grid gap-5 lg:grid-cols-[420px_1fr]">
        <Card>
          <CardHeader>
            <CardTitle>{editingId === null ? "Add checker" : "Edit checker"}</CardTitle>
            <CardDescription>
              Poll a URL and alert when its content or status code changes.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="space-y-4" onSubmit={submitForm}>
              <div className="space-y-2">
                <Label htmlFor="checker-name">Name</Label>
                <Input
                  id="checker-name"
                  value={form.name}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, name: event.target.value }))
                  }
                  placeholder="Status page"
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="checker-url">URL</Label>
                <Input
                  id="checker-url"
                  type="url"
                  value={form.url}
                  onChange={(event) =>
                    setForm((prev) => ({ ...prev, url: event.target.value }))
                  }
                  placeholder="https://example.com/status"
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="checker-type">Check type</Label>
                <Select
                  id="checker-type"
                  value={form.checkType}
                  onChange={(event) =>
                    setForm((prev) => ({
                      ...prev,
                      checkType: event.target.value as CheckType,
                    }))
                  }
                >
                  <option value="content_changed">Content changed</option>
                  <option value="status_code_changed">Status code changed</option>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="checker-interval">Schedule</Label>
                <div className="flex gap-2">
                  <Input
                    id="checker-interval"
                    type="number"
                    min={1}
                    step={1}
                    value={form.intervalValue}
                    onChange={(event) =>
                      setForm((prev) => ({
                        ...prev,
                        intervalValue: event.target.value,
                      }))
                    }
                  />
                  <Label htmlFor="checker-interval-unit" className="sr-only">
                    Schedule unit
                  </Label>
                  <Select
                    id="checker-interval-unit"
                    className="w-36"
                    value={form.intervalUnit}
                    onChange={(event) =>
                      setForm((prev) => ({
                        ...prev,
                        intervalUnit: event.target.value as IntervalUnit,
                      }))
                    }
                  >
                    <option value="seconds">Seconds</option>
                    <option value="minutes">Minutes</option>
                    <option value="hours">Hours</option>
                  </Select>
                </div>
                <p className="text-xs text-muted-foreground">
                  Minimum 60 seconds between runs.
                </p>
              </div>

              <div className="space-y-2">
                <Label htmlFor="checker-enabled">Enabled</Label>
                <Select
                  id="checker-enabled"
                  value={form.enabled ? "yes" : "no"}
                  onChange={(event) =>
                    setForm((prev) => ({
                      ...prev,
                      enabled: event.target.value === "yes",
                    }))
                  }
                >
                  <option value="yes">Yes</option>
                  <option value="no">No</option>
                </Select>
              </div>

              {form.checkType === "status_code_changed" ? (
                <>
                  <div className="space-y-2">
                    <Label htmlFor="checker-target-status">
                      Target status (optional)
                    </Label>
                    <Input
                      id="checker-target-status"
                      type="number"
                      min={100}
                      max={599}
                      value={form.targetStatus}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          targetStatus: event.target.value,
                        }))
                      }
                      placeholder="200"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="checker-mode">Alert mode</Label>
                    <Select
                      id="checker-mode"
                      value={form.mode}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          mode: event.target.value as StatusAlertMode,
                        }))
                      }
                    >
                      <option value="match">When status matches target</option>
                      <option value="mismatch">When status differs from target</option>
                      <option value="any">On any status</option>
                    </Select>
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="checker-alert-on-error-status">
                      Alert on error
                    </Label>
                    <Select
                      id="checker-alert-on-error-status"
                      value={form.alertOnError ? "yes" : "no"}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          alertOnError: event.target.value === "yes",
                        }))
                      }
                    >
                      <option value="no">No</option>
                      <option value="yes">Yes</option>
                    </Select>
                  </div>
                </>
              ) : (
                <>
                  <div className="space-y-2">
                    <Label htmlFor="checker-selector">CSS selector (optional)</Label>
                    <Input
                      id="checker-selector"
                      value={form.selector}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          selector: event.target.value,
                        }))
                      }
                      placeholder="#main, .content"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="checker-ignore-whitespace">
                      Ignore whitespace
                    </Label>
                    <Select
                      id="checker-ignore-whitespace"
                      value={form.ignoreWhitespace ? "yes" : "no"}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          ignoreWhitespace: event.target.value === "yes",
                        }))
                      }
                    >
                      <option value="yes">Yes</option>
                      <option value="no">No</option>
                    </Select>
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="checker-alert-on-error-content">
                      Alert on error
                    </Label>
                    <Select
                      id="checker-alert-on-error-content"
                      value={form.alertOnError ? "yes" : "no"}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          alertOnError: event.target.value === "yes",
                        }))
                      }
                    >
                      <option value="no">No</option>
                      <option value="yes">Yes</option>
                    </Select>
                  </div>
                </>
              )}

              <div className="flex gap-2">
                <Button disabled={saving}>
                  <Plus className="h-4 w-4" />
                  {editingId === null
                    ? saving
                      ? "Adding..."
                      : "Add checker"
                    : saving
                      ? "Saving..."
                      : "Save changes"}
                </Button>
                {editingId !== null ? (
                  <Button type="button" variant="outline" onClick={resetForm}>
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
              Loading checkers...
            </div>
          ) : checkers.length === 0 ? (
            <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
              No web checkers configured.
            </div>
          ) : (
            checkers.map((checker) => {
              const isBusy = busy.has(checker.id);
              return (
                <Card key={checker.id}>
                  <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
                    <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                      <Globe className="h-4 w-4 text-primary" />
                      <CardTitle className="truncate">{checker.name}</CardTitle>
                      <Badge variant="secondary">
                        {checkTypeLabel(checker.check_type)}
                      </Badge>
                      <Badge
                        variant={checker.enabled ? "success" : "secondary"}
                      >
                        {checker.enabled ? "Enabled" : "Disabled"}
                      </Badge>
                    </div>
                    <div className="flex shrink-0 flex-wrap justify-end gap-1">
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={isBusy}
                        onClick={() => void runChecker(checker)}
                      >
                        <Play className="h-3 w-3" />
                        Run now
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={isBusy || !checker.has_current_content}
                        title={
                          checker.has_current_content
                            ? undefined
                            : "No content snapshot available yet"
                        }
                        onClick={() => void toggleDiff(checker)}
                      >
                        <FileDiff className="h-3 w-3" />
                        Diff
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={isBusy}
                        onClick={() => toggleResults(checker)}
                      >
                        <History className="h-3 w-3" />
                        {results?.checkerId === checker.id ? "Hide" : "Results"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => startEdit(checker)}
                      >
                        <Pencil className="h-4 w-4" />
                        <span className="sr-only">Edit</span>
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        disabled={isBusy}
                        onClick={() => void deleteChecker(checker)}
                      >
                        <Trash2 className="h-4 w-4" />
                        <span className="sr-only">Delete</span>
                      </Button>
                    </div>
                  </CardHeader>

                  <CardContent>
                    <CardDescription className="break-all font-mono text-xs">
                      {checker.url}
                    </CardDescription>
                    <div className="mt-3 grid grid-cols-2 gap-x-6 gap-y-3 sm:grid-cols-4">
                      <Info
                        label="Last status"
                        value={formatStatusCode(checker.last_status_code)}
                      />
                      <Info
                        label="Schedule"
                        value={formatInterval(checker.interval_seconds)}
                      />
                      <Info
                        label="Last run"
                        value={formatDateTime(checker.last_run_at)}
                      />
                      <Info
                        label="Next run"
                        value={formatDateTime(checker.next_run_at)}
                      />
                    </div>
                    <div className="mt-3 text-xs text-muted-foreground">
                      Last changed {formatDateTime(checker.last_changed_at)}
                    </div>
                  </CardContent>

                  {diff?.checkerId === checker.id ? (
                    <CardContent className="border-t border-border pt-4">
                      <div className="mb-2 flex items-center justify-between">
                        <span className="text-sm font-medium">Content diff</span>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => setDiff(null)}
                        >
                          <X className="h-4 w-4" />
                          <span className="sr-only">Close diff</span>
                        </Button>
                      </div>
                      {diff.data.previous_content === null ? (
                        <p className="rounded-md border border-border bg-muted/30 px-3 py-4 text-sm text-muted-foreground">
                          No previous snapshot yet. The next change will be shown
                          here.
                        </p>
                      ) : (
                        <pre className="max-h-96 overflow-auto rounded-md border border-border bg-muted/30 py-2 text-xs leading-5">
                          {lineDiff(
                            diff.data.previous_content,
                            diff.data.current_content ?? "",
                          ).map((line, index) => (
                            <div
                              key={index}
                              className={`px-3 ${diffLineClass(line.type)}`}
                            >
                              <span className="select-none pr-2 font-mono">
                                {diffPrefix(line.type)}
                              </span>
                              <span className="whitespace-pre-wrap break-all font-mono">
                                {line.text === "" ? " " : line.text}
                              </span>
                            </div>
                          ))}
                        </pre>
                      )}
                    </CardContent>
                  ) : null}

                  {results?.checkerId === checker.id ? (
                    <CardContent className="border-t border-border pt-4">
                      <div className="mb-2 flex items-center gap-2 text-sm font-medium">
                        <History className="h-4 w-4 text-primary" />
                        Run history
                      </div>
                      {results.data.items.length === 0 ? (
                        <p className="rounded-md border border-border bg-muted/30 px-3 py-4 text-sm text-muted-foreground">
                          No runs recorded yet.
                        </p>
                      ) : (
                        <div className="overflow-x-auto rounded-md border border-border">
                          <table className="w-full min-w-[720px] border-collapse text-sm">
                            <thead>
                              <tr className="border-b border-border text-left text-muted-foreground">
                                <th className="px-3 py-2 font-medium">Started</th>
                                <th className="px-3 py-2 font-medium">Status</th>
                                <th className="px-3 py-2 font-medium">Triggered</th>
                                <th className="px-3 py-2 font-medium">Duration</th>
                                <th className="px-3 py-2 font-medium">Message</th>
                              </tr>
                            </thead>
                            <tbody>
                              {results.data.items.map((result) => (
                                <tr
                                  key={result.id}
                                  className="border-b border-border/70 align-top"
                                >
                                  <td className="px-3 py-2 text-muted-foreground">
                                    {formatDateTime(result.started_at)}
                                  </td>
                                  <td className="px-3 py-2">
                                    {result.status_code === null
                                      ? "-"
                                      : result.status_code}
                                  </td>
                                  <td className="px-3 py-2">
                                    <Badge
                                      variant={
                                        result.triggered
                                          ? "destructive"
                                          : "secondary"
                                      }
                                    >
                                      {result.triggered ? "alert" : "no alert"}
                                    </Badge>
                                  </td>
                                  <td className="px-3 py-2 text-muted-foreground">
                                    {formatDuration(result.duration_ms)}
                                  </td>
                                  <td className="max-w-md px-3 py-2 text-muted-foreground">
                                    {result.message}
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                      <div className="mt-3 flex items-center justify-between gap-2 text-sm text-muted-foreground">
                        <span>
                          Page {results.data.page} of{" "}
                          {Math.max(
                            1,
                            Math.ceil(
                              results.data.total / results.data.page_size,
                            ),
                          )}
                        </span>
                        <div className="flex gap-2">
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={isBusy || results.data.page <= 1}
                            onClick={() =>
                              void loadResults(checker, results.data.page - 1)
                            }
                          >
                            <ChevronLeft className="h-4 w-4" />
                            Previous
                          </Button>
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={
                              isBusy ||
                              results.data.page >=
                                Math.max(
                                  1,
                                  Math.ceil(
                                    results.data.total / results.data.page_size,
                                  ),
                                )
                            }
                            onClick={() =>
                              void loadResults(checker, results.data.page + 1)
                            }
                          >
                            Next
                            <ChevronRight className="h-4 w-4" />
                          </Button>
                        </div>
                      </div>
                    </CardContent>
                  ) : null}
                </Card>
              );
            })
          )}
        </div>
      </div>
    </main>
  );
}
