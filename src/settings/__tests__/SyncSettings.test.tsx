import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { SyncSettings } from "../SyncSettings";

describe("SyncSettings", () => {
  it("shows 'Not configured' and disables actions when no backend URL is set", () => {
    render(
      <SyncSettings
        backendUrl={null}
        status={null}
        pairingCode={null}
        onSaveBackendUrl={() => {}}
        onBootstrap={() => {}}
        onGenerateCode={() => {}}
        onJoin={() => {}}
        onSyncNow={() => {}}
      />,
    );
    expect(screen.getByText(/not configured/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /sync now/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /enable on this device/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /generate pairing code/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^join$/i })).toBeDisabled();
  });

  it("renders the status text and enables actions once a backend URL is configured", () => {
    render(
      <SyncSettings
        backendUrl="https://example.com"
        status={{ lastRunAt: "2026-08-21T00:00:00Z", outcome: "ok", pushed: 3, pulled: 5 }}
        pairingCode={null}
        onSaveBackendUrl={() => {}}
        onBootstrap={() => {}}
        onGenerateCode={() => {}}
        onJoin={() => {}}
        onSyncNow={() => {}}
      />,
    );
    expect(screen.getByText(/last sync: ok \(3 pushed, 5 pulled\)/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /sync now/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /enable on this device/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /generate pairing code/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^join$/i })).toBeEnabled();
  });

  it("calls onSaveBackendUrl with the typed URL when Save is clicked", () => {
    const fn = vi.fn();
    render(
      <SyncSettings
        backendUrl={null}
        status={null}
        pairingCode={null}
        onSaveBackendUrl={fn}
        onBootstrap={() => {}}
        onGenerateCode={() => {}}
        onJoin={() => {}}
        onSyncNow={() => {}}
      />,
    );
    fireEvent.change(screen.getByPlaceholderText(/your-sync-backend/i), {
      target: { value: "https://backend.example.com" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));
    expect(fn).toHaveBeenCalledWith("https://backend.example.com");
  });

  it("calls onSyncNow when Sync now is clicked", () => {
    const fn = vi.fn();
    render(
      <SyncSettings
        backendUrl="https://example.com"
        status={null}
        pairingCode={null}
        onSaveBackendUrl={() => {}}
        onBootstrap={() => {}}
        onGenerateCode={() => {}}
        onJoin={() => {}}
        onSyncNow={fn}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /sync now/i }));
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("renders the pairing code when provided", () => {
    render(
      <SyncSettings
        backendUrl="https://example.com"
        status={null}
        pairingCode="ABCD-1234"
        onSaveBackendUrl={() => {}}
        onBootstrap={() => {}}
        onGenerateCode={() => {}}
        onJoin={() => {}}
        onSyncNow={() => {}}
      />,
    );
    expect(screen.getByText("ABCD-1234")).toBeInTheDocument();
  });

  it("calls onJoin with the typed pairing code and device name when Join is clicked", () => {
    const fn = vi.fn();
    render(
      <SyncSettings
        backendUrl="https://example.com"
        status={null}
        pairingCode={null}
        onSaveBackendUrl={() => {}}
        onBootstrap={() => {}}
        onGenerateCode={() => {}}
        onJoin={fn}
        onSyncNow={() => {}}
      />,
    );
    fireEvent.change(screen.getByPlaceholderText(/pairing code/i), {
      target: { value: "WXYZ-5678" },
    });
    fireEvent.change(screen.getByPlaceholderText(/this device's name/i), {
      target: { value: "My Laptop" },
    });
    fireEvent.click(screen.getByRole("button", { name: /^join$/i }));
    expect(fn).toHaveBeenCalledWith("WXYZ-5678", "My Laptop");
  });
});
