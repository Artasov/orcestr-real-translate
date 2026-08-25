import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import {
  applyAuthSnapshotToCurrentUser,
  CURRENT_USER_QUERY_KEY,
} from "./auth-session";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("native auth session query synchronization", () => {
  it("lets a pending current-user request settle after signed-out event", async () => {
    const currentUser = deferred<never>();
    const queryFn = vi.fn(() => currentUser.promise);
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const observer = new QueryObserver(queryClient, {
      queryKey: CURRENT_USER_QUERY_KEY,
      queryFn,
      retry: false,
    });
    const unsubscribe = observer.subscribe(() => undefined);

    expect(observer.getCurrentResult()).toMatchObject({
      status: "pending",
      fetchStatus: "fetching",
    });

    applyAuthSnapshotToCurrentUser(queryClient, {
      phase: "signedOut",
      profile: null,
      message: null,
    });
    currentUser.reject(new Error("not_authenticated"));
    await vi.waitFor(() => {
      expect(observer.getCurrentResult()).toMatchObject({
        status: "error",
        fetchStatus: "idle",
        data: null,
      });
    });

    expect(queryFn).toHaveBeenCalledTimes(1);
    unsubscribe();
  });

  it("publishes authenticated and external signed-out states without refetching", () => {
    const queryClient = new QueryClient();
    const profile = { id: "42", username: "user", email: "user@example.test" };

    applyAuthSnapshotToCurrentUser(queryClient, {
      phase: "authenticated",
      profile,
      message: null,
    });
    expect(queryClient.getQueryData(CURRENT_USER_QUERY_KEY)).toEqual(profile);

    applyAuthSnapshotToCurrentUser(queryClient, {
      phase: "signedOut",
      profile: null,
      message: null,
    });
    expect(queryClient.getQueryData(CURRENT_USER_QUERY_KEY)).toBeNull();
  });

  it("does not overwrite the current user for transient native phases", () => {
    const queryClient = new QueryClient();
    const profile = { id: "42", username: "user" };
    queryClient.setQueryData(CURRENT_USER_QUERY_KEY, profile);

    for (const phase of ["bootstrapping", "authorizing"] as const) {
      applyAuthSnapshotToCurrentUser(queryClient, {
        phase,
        profile: null,
        message: null,
      });
    }

    expect(queryClient.getQueryData(CURRENT_USER_QUERY_KEY)).toEqual(profile);
  });
});
