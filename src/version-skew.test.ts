import { describe, expect, it } from "vitest";
import { compareHerdrVersions } from "./version-skew";

describe("compareHerdrVersions", () => {
  it("equal is ok", () => {
    const r = compareHerdrVersions("0.9.5", "0.9.5");
    expect(r.ok).toBe(true);
    expect(r.severity).toBe("none");
  });
  it("tolerates v prefix and pre-release suffix", () => {
    expect(compareHerdrVersions("v0.9.5", "0.9.5-beta.1").ok).toBe(true);
  });
  it("patch differs = warn", () => {
    const r = compareHerdrVersions("0.9.5", "0.9.6");
    expect(r.ok).toBe(false);
    expect(r.severity).toBe("warn");
  });
  it("minor differs = error", () => {
    expect(compareHerdrVersions("0.9.5", "0.10.5").severity).toBe("error");
  });
  it("major differs = error", () => {
    expect(compareHerdrVersions("1.0.0", "0.9.5").severity).toBe("error");
  });
  it("unparsable = warn", () => {
    expect(compareHerdrVersions("garbage", "0.9.5").severity).toBe("warn");
    expect(compareHerdrVersions("0.9.5", "").severity).toBe("warn");
  });
});
