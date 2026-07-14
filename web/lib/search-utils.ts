/**
 * search-utils.ts — shared keyword-search utilities for docs and FAQ.
 *
 * Pure functions extracted from the client components so they can be unit-tested
 * without a DOM. Used by DocsSearch and FaqSearch.
 */

import type { DocTopic } from "./docs-map";

const CATEGORY_LABELS: Record<string, { en: string; zh: string }> = {
  "getting-started": { en: "Getting started", zh: "入门" },
  "core-concepts": { en: "Core concepts", zh: "核心概念" },
  reference: { en: "Reference", zh: "参考" },
  extending: { en: "Extending", zh: "扩展" },
  operations: { en: "Operations & community", zh: "运维与社区" },
};

/**
 * Build a lowercase haystack string for a DocTopic, searching across both
 * locales, source files, category name, and id/slug.
 */
export function docTopicHaystack(t: DocTopic): string {
  const sources = Array.isArray(t.repoSource) ? t.repoSource : [t.repoSource];
  const parts = [
    t.id,
    t.slug,
    t.label.en,
    t.label.zh,
    t.description.en,
    t.description.zh,
    ...sources,
    t.category,
    CATEGORY_LABELS[t.category]?.en ?? "",
    CATEGORY_LABELS[t.category]?.zh ?? "",
  ];
  return parts.join(" ").toLowerCase();
}

/**
 * Filter DocTopics by keyword query. Returns indices into the input array.
 * Empty/whitespace query returns all indices.
 */
export function filterDocTopics(topics: DocTopic[], query: string): number[] {
  const q = query.trim().toLowerCase();
  if (!q) return topics.map((_, i) => i);
  return topics
    .map((t, i) => ({ i, hay: docTopicHaystack(t) }))
    .filter(({ hay }) => hay.includes(q))
    .map(({ i }) => i);
}

/**
 * Normalize a query for matching.
 */
export function normalizeQuery(query: string): string {
  return query.trim().toLowerCase();
}

/**
 * Check whether a query matches a haystack (case-insensitive substring).
 */
export function matches(haystack: string, query: string): boolean {
  const q = normalizeQuery(query);
  if (!q) return true;
  return haystack.toLowerCase().includes(q);
}
