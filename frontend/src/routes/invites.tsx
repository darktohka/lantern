import { Copy, Plus, RefreshCw, Ticket, Trash2 } from "lucide-react";
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

type Invite = {
  id: number;
  code: string;
  created_at: string;
  redeemed_at: string | null;
  redeemed_by_username: string | null;
};

export function InvitesPage() {
  const { token } = useAuth();
  const [invites, setInvites] = useState<Invite[]>([]);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const unredeemedCount = invites.filter((invite) => !invite.redeemed_at).length;

  async function loadInvites() {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      setInvites(await apiFetch<Invite[]>("/api/invites", token));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load invites");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadInvites();
  }, [token]);

  async function createInvite() {
    if (!token) return;
    setCreating(true);
    setError(null);
    try {
      const invite = await apiFetch<Invite>("/api/invites", token, {
        method: "POST",
      });
      setInvites((current) => [invite, ...current]);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create invite");
    } finally {
      setCreating(false);
    }
  }

  async function copyInvite(code: string) {
    await navigator.clipboard.writeText(code);
  }

  async function deleteInvite(invite: Invite) {
    if (!token) return;
    if (!window.confirm(`Delete invite code "${invite.code}"?`)) return;

    setError(null);
    try {
      await apiFetch<void>(`/api/invites/${invite.id}`, token, {
        method: "DELETE",
      });
      setInvites((current) => current.filter((i) => i.id !== invite.id));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete invite");
    }
  }

  return (
    <main className="mx-auto max-w-6xl px-4 py-6">
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-normal">Invites</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {unredeemedCount} of 5 unredeemed codes
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" onClick={() => void loadInvites()}>
            <RefreshCw className="h-4 w-4" />
            Refresh
          </Button>
          <Button
            onClick={() => void createInvite()}
            disabled={creating || unredeemedCount >= 5}
          >
            <Plus className="h-4 w-4" />
            Generate
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
            <Ticket className="h-5 w-5 text-primary" />
            <CardTitle>Invite codes</CardTitle>
          </div>
          <CardDescription>Unredeemed invite codes are capped per user.</CardDescription>
        </CardHeader>
        <CardContent>
          {loading ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              Loading invites...
            </div>
          ) : invites.length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              No invites generated.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[720px] border-collapse text-sm">
                <thead>
                  <tr className="border-b border-border text-left text-muted-foreground">
                    <th className="py-3 pr-4 font-medium">Code</th>
                    <th className="py-3 pr-4 font-medium">Status</th>
                    <th className="py-3 pr-4 font-medium">Created</th>
                    <th className="py-3 pr-4 font-medium">Redeemed by</th>
                    <th className="py-3 text-right font-medium"></th>
                  </tr>
                </thead>
                <tbody>
                  {invites.map((invite) => (
                    <tr key={invite.id} className="border-b border-border/70">
                      <td className="py-3 pr-4 font-mono text-xs">{invite.code}</td>
                      <td className="py-3 pr-4">
                        <Badge
                          variant={invite.redeemed_at ? "secondary" : "success"}
                        >
                          {invite.redeemed_at ? "Redeemed" : "Open"}
                        </Badge>
                      </td>
                      <td className="py-3 pr-4 text-muted-foreground">
                        {formatDateTime(invite.created_at)}
                      </td>
                      <td className="py-3 pr-4 text-muted-foreground">
                        {invite.redeemed_by_username ?? "-"}
                      </td>
                      <td className="py-3 text-right">
                        <div className="flex justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => void copyInvite(invite.code)}
                          >
                            <Copy className="h-4 w-4" />
                            <span className="sr-only">Copy</span>
                          </Button>
                          {!invite.redeemed_at ? (
                            <Button
                              variant="ghost"
                              size="icon"
                              onClick={() => void deleteInvite(invite)}
                            >
                              <Trash2 className="h-4 w-4" />
                              <span className="sr-only">Delete</span>
                            </Button>
                          ) : null}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </main>
  );
}
