# Module 11: Web Search and Research

## Responsibility

Give the agent source-grounded research capability for current information, long-running investigations, package/library checks, and evidence-based plans.

## Rust Crate

`crates/sakha-research`

## Main Structs

- `SearchClient`
- `SearchProvider`
- `SearchPlan`
- `SearchQuery`
- `SearchResult`
- `FetchRequest`
- `FetchedPage`
- `ExtractedDocument`
- `SourceScore`
- `EvidencePack`
- `Citation`
- `ResearchLoopState`

## Search Providers

- General web search API.
- Domain-restricted search.
- GitHub search.
- Package registry search.
- Documentation site search.
- Internal docs connector.

## Research Loop

1. Convert task into research questions.
2. Generate search queries.
3. Search multiple providers.
4. Deduplicate results.
5. Fetch highest quality pages.
6. Extract text and metadata.
7. Score sources.
8. Compress documents.
9. Build evidence pack.
10. Verify answer against evidence.
11. Store research memory.

## Source Scoring

Signals:

- Primary source.
- Official documentation.
- Publication date.
- Author/source reputation.
- Direct relevance.
- Contains code/API examples.
- Conflicts with other sources.
- Paywall/dynamic content risk.
- Content freshness.

## Long-Running Research

For long-running loops:

- Keep a research notebook.
- Track visited URLs.
- Track stale claims.
- Re-search unstable facts.
- Refresh package/API docs before implementation.
- Store evidence packs with goal IDs.
- Emit "research outdated" warnings.

## Web Search Safety

- Never execute downloaded code.
- Treat web content as untrusted.
- Strip prompt injection instructions.
- Separate source text from agent instructions.
- Cite sources in user-facing answers.
- Cache pages with timestamp and URL.

## Implementation Tasks

1. Define search provider trait.
2. Implement query planner.
3. Implement fetch/extract pipeline.
4. Implement source scorer.
5. Implement evidence pack schema.
6. Integrate Headroom compression for fetched docs.
7. Implement research memory.
8. Implement citation formatter.
9. Implement loop stop conditions.
10. Implement injection filter.

## Tests

- Query planning for library docs.
- Source scoring prioritizes official docs.
- Duplicate URL detection.
- Prompt injection text is quarantined.
- Evidence pack cites fetched sources.
- Long-running loop refreshes stale source.

