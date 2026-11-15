import { describe, expect, it } from "vitest";
import { compareHerdrVersions, skewBanners } from "./version-skew";

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

describe("skewBanners", () => {
  const row = (l: string | null, p: string | null, id = "m") => ({ machine_id: id, peer_label: "Mac", local_herdr_version: l, peer_herdr_version: p });
  it("null on either side = no banner", () => {
    expect(skewBanners([row(null, "0.9.5"), row("0.9.5", null), row(null, null)])).toEqual([]);
  });
  it("equal = no banner", () => {
    expect(skewBanners([row("0.9.5", "0.9.5")])).toEqual([]);
  });
  it("patch diff = warn, minor diff = error, per machine", () => {
    const b = skewBanners([row("0.9.5", "0.9.6", "a"), row("0.9.5", "0.10.0", "b")]);
    expect(b.map((x) => [x.machine_id, x.severity])).toEqual([["a", "warn"], ["b", "error"]]);
  });
});
