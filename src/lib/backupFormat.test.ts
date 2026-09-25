import { describe, expect, it } from "vitest";
import { displaySnapshotLabel, formatBytes, formatSnapshotWhen } from "./backupFormat";

describe("displaySnapshotLabel", () => {
  it("strips the sm-v- prefix and the trailing suffix", () => {
    expect(displaySnapshotLabel("sm-v-20250102-030405-ab12")).toBe("20250102-030405");
  });

  it("keeps short tags as they are, minus the prefix", () => {
    expect(displaySnapshotLabel("sm-v-20250102")).toBe("20250102");
    expect(displaySnapshotLabel("safety-point")).toBe("safety-point");
  });
});

describe("formatSnapshotWhen", () => {
  it("turns YYYYMMDD-HHMMSS into YYYY-MM-DD HH:MM", () => {
    expect(formatSnapshotWhen("sm-v-20250102-030405-ab12")).toBe("2025-01-02 03:04");
  });

  it("falls back to the label when it has no timestamp", () => {
    expect(formatSnapshotWhen("sm-v-latest")).toBe("latest");
  });

  it("is null without a tag", () => {
    expect(formatSnapshotWhen(null)).toBeNull();
  });
});

describe("formatBytes", () => {
  it("shows at least 1 KB", () => {
    expect(formatBytes(0)).toBe("1 KB");
    expect(formatBytes(300)).toBe("1 KB");
    expect(formatBytes(10 * 1024)).toBe("10 KB");
  });

  it("shows whole megabytes", () => {
    expect(formatBytes(5.4 * 1024 ** 2)).toBe("5 MB");
  });

  it("shows gigabytes with one decimal", () => {
    expect(formatBytes(1.25 * 1024 ** 3)).toBe("1.3 GB");
  });
});
