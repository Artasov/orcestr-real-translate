import { ApiError } from "@orcestr/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import { TauriAuthClient, isAllowedLegalDocumentUrl, nativeAuth } from "./auth";

const user = {
  id: 42,
  username: "translator@example.test",
  email: "translator@example.test",
};

describe("typed native authentication transport", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
  });

  it("submits password login and only the exact legal acceptance payload", async () => {
    invokeMock.mockResolvedValueOnce(user);
    const client = new TauriAuthClient();

    await expect(
      client.login("translator@example.test", "secret-password", {
        accepted_legal_documents: [
          {
            document_slug: "privacy-policy",
            version: "1.2",
            language: "en",
          },
        ],
      }),
    ).resolves.toEqual({ user });

    expect(invokeMock).toHaveBeenCalledWith("auth_login", {
      request: {
        username: "translator@example.test",
        password: "secret-password",
        acceptedLegalDocuments: [
          {
            documentSlug: "privacy-policy",
            version: "1.2",
            language: "en",
          },
        ],
      },
    });
    expect(JSON.stringify(invokeMock.mock.calls)).not.toContain("access_token");
    expect(JSON.stringify(invokeMock.mock.calls)).not.toContain(
      "refresh_token",
    );
  });

  it("rejects renderer payload extensions before invoking Rust", async () => {
    const client = new TauriAuthClient();

    await expect(
      client.login("translator@example.test", "secret-password", {
        redirect_uri: "https://attacker.test",
      }),
    ).rejects.toMatchObject({
      name: "ApiError",
      status: 400,
      code: "invalid_native_request",
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("uses production-origin methods and refuses an arbitrary origin", async () => {
    const methods = {
      email_password_allowed: true,
      allowed_oauth_providers: ["google", "github"],
      oauth_client_ids: { google: "google-client", github: "github-client" },
      country_known: true,
      allowed_email_domains: [],
    };
    invokeMock.mockResolvedValueOnce(methods);
    const client = new TauriAuthClient();

    await expect(client.methods("https://orcestr.com")).resolves.toEqual(
      methods,
    );
    expect(invokeMock).toHaveBeenCalledWith("auth_methods", undefined);
    invokeMock.mockClear();
    await expect(client.methods("https://attacker.test")).rejects.toMatchObject(
      {
        name: "ApiError",
        code: "oauth_origin_not_allowed",
      },
    );
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("turns structured native failures into the shared ApiError", async () => {
    invokeMock.mockRejectedValueOnce({
      status: 422,
      code: "invalid_credentials",
      message: "Invalid credentials",
      params: { attempts: 2 },
      fields: [
        {
          path: ["password", 0],
          code: "invalid",
          params: { min_length: 8 },
        },
      ],
      requestId: "req-1",
    });

    const error = await new TauriAuthClient()
      .me()
      .catch((value: unknown) => value);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({
      status: 422,
      code: "invalid_credentials",
      params: { attempts: 2 },
      requestId: "req-1",
    });
    expect((error as ApiError).fields).toEqual([
      {
        path: ["password", 0],
        code: "invalid",
        params: { min_length: 8 },
      },
    ]);
  });

  it("does not trust arbitrary JSON inside a rejected IPC payload", async () => {
    invokeMock.mockRejectedValueOnce({
      status: 422,
      code: "invalid_credentials",
      message: "Invalid credentials",
      fields: [{ path: [{ secret: true }], code: "invalid" }],
    });

    await expect(new TauriAuthClient().me()).rejects.toMatchObject({
      name: "ApiError",
      status: 0,
      code: "network_error",
      fields: [],
    });
  });

  it("starts provider OAuth through one typed command without browser URL input", async () => {
    invokeMock.mockResolvedValueOnce({
      phase: "authorizing",
      profile: null,
      message: null,
    });

    await nativeAuth.beginOAuth("google");

    expect(invokeMock).toHaveBeenCalledWith("auth_begin_oauth", {
      provider: "google",
    });
  });

  it("cancels the native OAuth attempt without renderer state or URL input", async () => {
    invokeMock.mockResolvedValueOnce({
      phase: "signedOut",
      profile: null,
      message: null,
    });

    await nativeAuth.cancelOAuth();

    expect(invokeMock).toHaveBeenCalledWith("auth_cancel_oauth", undefined);
  });
});

describe("legal-document browser allowlist", () => {
  it.each([
    "https://orcestr.com/legal/privacy-policy",
    "https://orcestr.com/ru/legal/user-agreement",
  ])("allows %s", (value) => {
    expect(isAllowedLegalDocumentUrl(value)).toBe(true);
  });

  it.each([
    "http://orcestr.com/legal/privacy-policy",
    "https://www.orcestr.com/legal/privacy-policy",
    "https://orcestr.com.attacker.test/legal/privacy-policy",
    "https://orcestr.com/legal/privacy-policy?next=https://attacker.test",
    "https://orcestr.com/legal/privacy-policy#section",
    "https://orcestr.com/ru/legal/privacy-policy/extra",
  ])("rejects %s", (value) => {
    expect(isAllowedLegalDocumentUrl(value)).toBe(false);
  });
});
