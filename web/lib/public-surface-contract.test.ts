import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { FACTS } from "./facts.generated";

const root = new URL("../../", import.meta.url);
const matrix = JSON.parse(
  readFileSync(new URL("docs/public-surface-facts.json", root), "utf8"),
) as {
  release: { version: string; providerCount: number };
  screenshot: {
    readme: string;
    website: string;
    sourceVersion: string;
    sourceCommit: string;
    terminal: string;
    sources: string[];
  };
  [key: string]: unknown;
};

function bytes(path: string): Buffer {
  return readFileSync(new URL(path, root));
}

function pngDimensions(image: Buffer): [number, number] {
  expect(image.subarray(1, 4).toString("ascii")).toBe("PNG");
  return [image.readUInt32BE(16), image.readUInt32BE(20)];
}

describe("public surface contracts", () => {
  it("keeps checked-in release facts aligned with generated repository facts", () => {
    expect(matrix.release.version).toBe(FACTS.version);
    expect(matrix.release.providerCount).toBe(FACTS.providers.length);
    expect(matrix.screenshot.sourceVersion).toBe(FACTS.version);
    expect(matrix.screenshot.sourceCommit).toMatch(/^[0-9a-f]{40}$/);
    expect(matrix.screenshot.terminal).toBe("106x32");
  });

  it("keeps the README and website on one optimized canonical product screenshot", () => {
    const readmeImage = bytes(matrix.screenshot.readme);
    const websiteImage = bytes(matrix.screenshot.website);
    const digest = (image: Buffer) => createHash("sha256").update(image).digest("hex");

    expect(digest(readmeImage)).toBe(digest(websiteImage));
    expect(pngDimensions(readmeImage)).toEqual([1280, 720]);
    expect(statSync(new URL(matrix.screenshot.readme, root)).size).toBeLessThan(500_000);

    const readme = readFileSync(new URL("README.md", root), "utf8");
    const homepage = readFileSync(new URL("web/app/[locale]/page.tsx", root), "utf8");
    expect(readme).toContain("assets/screenshot.png");
    expect(homepage).toContain('src="/codewhale-tui.png"');
    expect(homepage).toContain("with no empty Work bar");
  });

  it("keeps every fact-matrix source resolvable in the repository", () => {
    const sources = new Set<string>();
    const visit = (value: unknown) => {
      if (Array.isArray(value)) {
        value.forEach(visit);
      } else if (value && typeof value === "object") {
        for (const [key, child] of Object.entries(value)) {
          if (key === "sources" || key === "creditSources") {
            (child as string[]).forEach((source) => sources.add(source));
          } else {
            visit(child);
          }
        }
      }
    };
    visit(matrix);

    for (const source of sources) {
      expect(existsSync(new URL(source, root)), source).toBe(true);
    }
  });
});
