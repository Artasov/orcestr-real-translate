import type { QueryClient } from "@tanstack/react-query";

import type { AuthSnapshot, OrcestrUser } from "./auth";

export const CURRENT_USER_QUERY_KEY = ["orcestr-auth", "current-user"] as const;

/**
 * Mirrors terminal native auth state into the shared auth query without
 * removing an active query. Removing a pending query detaches its observer
 * from the in-flight `auth_me` promise and leaves React Query fetching forever.
 */
export function applyAuthSnapshotToCurrentUser(
  queryClient: QueryClient,
  snapshot: AuthSnapshot,
): void {
  if (snapshot.phase === "authenticated" && snapshot.profile) {
    queryClient.setQueryData<OrcestrUser | null>(
      CURRENT_USER_QUERY_KEY,
      snapshot.profile,
    );
    return;
  }

  if (snapshot.phase === "signedOut" || snapshot.phase === "error") {
    queryClient.setQueryData<OrcestrUser | null>(CURRENT_USER_QUERY_KEY, null);
  }
}
