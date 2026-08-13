// src/accounts/__tests__/ScheduleSelector.test.tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ScheduleSelector, type Schedule } from "../ScheduleSelector";

describe("ScheduleSelector", () => {
  it("renders three preset radio options", () => {
    render(
      <ScheduleSelector value={{ type: "Off" }} onChange={() => {}} />,
    );
    expect(screen.getByRole("radio", { name: /off/i })).toBeChecked();
    expect(screen.getByRole("radio", { name: /every 5h/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /custom/i })).toBeInTheDocument();
  });

  it("switching to Every5h emits a default 06:00 anchor", () => {
    const fn = vi.fn();
    render(
      <ScheduleSelector value={{ type: "Off" }} onChange={fn} />,
    );
    fireEvent.click(screen.getByRole("radio", { name: /every 5h/i }));
    expect(fn).toHaveBeenCalledWith({
      type: "Every5h",
      anchor: { hour: 6, minute: 0 },
    } as Schedule);
  });

  it("Custom mode shows an empty list with an Add button", () => {
    render(
      <ScheduleSelector
        value={{ type: "Custom", times: [] }}
        onChange={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: /add time/i })).toBeInTheDocument();
  });

  it("Adding a time emits a Custom value with one entry", () => {
    const fn = vi.fn();
    render(
      <ScheduleSelector
        value={{ type: "Custom", times: [] }}
        onChange={fn}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /add time/i }));
    expect(fn).toHaveBeenCalledWith({
      type: "Custom",
      times: [{ hour: 9, minute: 0 }],
    } as Schedule);
  });

  describe("warm-up anchor suggestion", () => {
    const suggestion = { anchor: { hour: 8, minute: 30 }, activeDays: 14 };

    it("renders nothing extra when no suggestion is provided", () => {
      render(<ScheduleSelector value={{ type: "Off" }} onChange={() => {}} />);
      expect(screen.queryByText(/usually start/i)).toBeNull();
    });

    it("shows the suggestion when schedule is Off", () => {
      render(
        <ScheduleSelector value={{ type: "Off" }} onChange={() => {}} suggestion={suggestion} />,
      );
      expect(screen.getByText(/usually start around 08:30/i)).toBeInTheDocument();
    });

    it("shows the suggestion when Every5h's anchor differs from it", () => {
      render(
        <ScheduleSelector
          value={{ type: "Every5h", anchor: { hour: 6, minute: 0 } }}
          onChange={() => {}}
          suggestion={suggestion}
        />,
      );
      expect(screen.getByText(/usually start around 08:30/i)).toBeInTheDocument();
    });

    it("hides the suggestion once Every5h's anchor already matches it", () => {
      render(
        <ScheduleSelector
          value={{ type: "Every5h", anchor: { hour: 8, minute: 30 } }}
          onChange={() => {}}
          suggestion={suggestion}
        />,
      );
      expect(screen.queryByText(/usually start/i)).toBeNull();
    });

    it("clicking Apply sets Every5h with the suggested anchor", () => {
      const fn = vi.fn();
      render(
        <ScheduleSelector value={{ type: "Off" }} onChange={fn} suggestion={suggestion} />,
      );
      fireEvent.click(screen.getByRole("button", { name: /apply/i }));
      expect(fn).toHaveBeenCalledWith({
        type: "Every5h",
        anchor: { hour: 8, minute: 30 },
      } as Schedule);
    });
  });
});
