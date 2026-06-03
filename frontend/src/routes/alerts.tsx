import { Bell, Plus, RefreshCw, Send, Trash2 } from "lucide-react";
import { FormEvent, useEffect, useState } from "react";

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

type NtfyAlertAuth =
  | { type: "anonymous" }
  | { type: "basic"; username: string; password: string }
  | { type: "bearer"; token: string };

type NtfyAlert = {
  id: number;
  name: string;
  topic: string;
  auth: NtfyAlertAuth;
  created_at: string;
};

const defaultAuth: NtfyAlertAuth = { type: "anonymous" };

export function AlertsPage() {
  const { token } = useAuth();
  const [alerts, setAlerts] = useState<NtfyAlert[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState<Set<number>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [topic, setTopic] = useState("");
  const [authType, setAuthType] = useState<"anonymous" | "basic" | "bearer">("anonymous");
  const [authUsername, setAuthUsername] = useState("");
  const [authPassword, setAuthPassword] = useState("");
  const [authToken, setAuthToken] = useState("");

  async function loadAlerts() {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      setAlerts(await apiFetch<NtfyAlert[]>("/api/ntfy-alerts", token));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load alerts");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadAlerts();
  }, [token]);

  function resetForm() {
    setName("");
    setTopic("");
    setAuthType("anonymous");
    setAuthUsername("");
    setAuthPassword("");
    setAuthToken("");
  }

  function buildAuth(): NtfyAlertAuth {
    switch (authType) {
      case "anonymous":
        return { type: "anonymous" };
      case "basic":
        return { type: "basic", username: authUsername, password: authPassword };
      case "bearer":
        return { type: "bearer", token: authToken };
    }
  }

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    if (!token) return;

    setSaving(true);
    setError(null);
    try {
      await apiFetch<NtfyAlert>("/api/ntfy-alerts", token, {
        method: "POST",
        body: JSON.stringify({ name, topic, auth: buildAuth() }),
      });
      resetForm();
      await loadAlerts();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create alert");
    } finally {
      setSaving(false);
    }
  }

  async function deleteAlert(alert: NtfyAlert) {
    if (!token) return;
    if (!window.confirm(`Delete alert "${alert.name}"?`)) return;

    setError(null);
    try {
      await apiFetch<void>(`/api/ntfy-alerts/${alert.id}`, token, {
        method: "DELETE",
      });
      await loadAlerts();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete alert");
    }
  }

  async function testAlert(alert: NtfyAlert) {
    if (!token) return;
    setTesting((prev) => new Set(prev).add(alert.id));
    setError(null);
    try {
      const result = await apiFetch<{ message: string }>(
        `/api/ntfy-alerts/${alert.id}/test`,
        token,
        { method: "POST" },
      );
      window.alert(result.message);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Test failed");
    } finally {
      setTesting((prev) => {
        const next = new Set(prev);
        next.delete(alert.id);
        return next;
      });
    }
  }

  function authLabel(auth: NtfyAlertAuth): string {
    switch (auth.type) {
      case "anonymous":
        return "No auth";
      case "basic":
        return "Basic auth";
      case "bearer":
        return "Bearer token";
    }
  }

  return (
    <main className="mx-auto max-w-6xl px-4 py-6">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">ntfy Alerts</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {alerts.length} alert{alerts.length !== 1 ? "s" : ""} configured
          </p>
        </div>
        <Button variant="outline" onClick={() => void loadAlerts()}>
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
            <CardTitle>Add alert</CardTitle>
            <CardDescription>
              When a check-in fails, a notification will be sent to the ntfy topic.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="space-y-4" onSubmit={submitForm}>
              <div className="space-y-2">
                <Label htmlFor="alert-name">Name</Label>
                <Input
                  id="alert-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="My phone"
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="alert-topic">Topic / URL</Label>
                <Input
                  id="alert-topic"
                  value={topic}
                  onChange={(event) => setTopic(event.target.value)}
                  placeholder="my_checkin_alerts"
                  required
                />
                <p className="text-xs text-muted-foreground">
                  A plain topic name (e.g. my_alerts) or a full ntfy URL.
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="alert-auth-type">Authentication</Label>
                <Select
                  id="alert-auth-type"
                  value={authType}
                  onChange={(event) => setAuthType(event.target.value as "anonymous" | "basic" | "bearer")}
                >
                  <option value="anonymous">None (anonymous)</option>
                  <option value="basic">Username &amp; password</option>
                  <option value="bearer">Access token</option>
                </Select>
              </div>

              {authType === "basic" ? (
                <>
                  <div className="space-y-2">
                    <Label htmlFor="auth-username">Username</Label>
                    <Input
                      id="auth-username"
                      value={authUsername}
                      onChange={(event) => setAuthUsername(event.target.value)}
                      placeholder="ntfy user"
                      autoComplete="off"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="auth-password">Password</Label>
                    <Input
                      id="auth-password"
                      type="password"
                      value={authPassword}
                      onChange={(event) => setAuthPassword(event.target.value)}
                      placeholder="********"
                      autoComplete="off"
                    />
                  </div>
                </>
              ) : authType === "bearer" ? (
                <div className="space-y-2">
                  <Label htmlFor="auth-token">Token</Label>
                  <Input
                    id="auth-token"
                    value={authToken}
                    onChange={(event) => setAuthToken(event.target.value)}
                    placeholder="tk_AgQdq7mVBoFD37zQVN29RhuMzNIz2"
                    autoComplete="off"
                  />
                </div>
              ) : null}

              <Button disabled={saving}>
                <Plus className="h-4 w-4" />
                {saving ? "Adding..." : "Add alert"}
              </Button>
            </form>
          </CardContent>
        </Card>

        <div className="space-y-3">
          {loading ? (
            <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
              Loading alerts...
            </div>
          ) : alerts.length === 0 ? (
            <div className="rounded-md border border-border bg-card px-4 py-8 text-center text-sm text-muted-foreground">
              No ntfy alerts configured.
            </div>
          ) : (
            alerts.map((alert) => (
              <Card key={alert.id}>
                <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <Bell className="h-4 w-4 text-primary" />
                      <CardTitle className="truncate">{alert.name}</CardTitle>
                      <Badge variant="secondary">ntfy</Badge>
                      <Badge variant="secondary">{authLabel(alert.auth)}</Badge>
                    </div>
                    <CardDescription className="mt-1">
                      Topic: <span className="font-mono text-xs">{alert.topic}</span>
                    </CardDescription>
                    <CardDescription>
                      Created {formatDateTime(alert.created_at)}
                    </CardDescription>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={testing.has(alert.id)}
                      onClick={() => void testAlert(alert)}
                    >
                      <Send className="h-3 w-3" />
                      {testing.has(alert.id) ? "Sending..." : "Test"}
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => void deleteAlert(alert)}
                    >
                      <Trash2 className="h-4 w-4" />
                      <span className="sr-only">Delete</span>
                    </Button>
                  </div>
                </CardHeader>
              </Card>
            ))
          )}
        </div>
      </div>
    </main>
  );
}