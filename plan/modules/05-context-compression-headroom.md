# Module 05: Context Compression and Headroom Integration

## Responsibility

Integrate Headroom as a first-class context optimization layer while keeping a provider-neutral compression abstraction.

## Rust Crate

`crates/sakha-compression`

## Headroom Integration Modes

1. **Library mode**: call Headroom library bindings if available.
2. **Proxy mode**: route provider calls through `headroom proxy`.
3. **MCP mode**: expose/use Headroom MCP tools:
   - `headroom_compress`
   - `headroom_retrieve`
   - `headroom_stats`
4. **Sidecar mode**: run Headroom as a managed local service.

## Main Structs

- `CompressionManager`
- `CompressionPolicy`
- `CompressionRoute`
- `HeadroomClient`
- `HeadroomSidecar`
- `ContextClassifier`
- `CompressionMarker`
- `RetrievalStore`
- `CompressionStats`
- `CompressionDecision`
- `CompressionAudit`

## Content Kinds

- `ConversationHistory`
- `ToolOutput`
- `ShellLog`
- `BuildLog`
- `TestLog`
- `JsonData`
- `CodeFile`
- `FileTree`
- `SearchResult`
- `WebPage`
- `RagChunk`
- `ErrorTrace`
- `Plan`
- `Handoff`

## Compression Pipeline

```text
ContextItem
  -> classify content kind
  -> determine compression policy
  -> Headroom route or local fallback
  -> write raw artifact
  -> write compressed artifact
  -> insert retrieval marker
  -> send compressed bundle to model
```

## Policy Fields

- `enabled`
- `mode`
- `min_tokens_to_compress`
- `max_lossiness`
- `reversible_required`
- `allowed_content_kinds`
- `blocked_content_kinds`
- `retrieve_on_model_request`
- `store_raw_for_days`
- `redact_before_compress`
- `fail_open_or_closed`

## Reversible CCR Requirements

- Raw content is stored locally.
- Compressed content contains stable marker IDs.
- Model can request retrieval through a tool.
- Retrieval is permissioned and audited.
- Retrieved content is compressed again if too large.

## Sakha-Specific Additions

- Compression A/B evals.
- Over-compression detector.
- Retrieval-rate metric.
- Context rot monitor.
- Cache-aligned prompt prefix builder.
- Compression diff viewer in UI.
- Compression bypass per task/security policy.

## Implementation Tasks

1. Define compression trait.
2. Implement Headroom sidecar launcher.
3. Implement Headroom MCP client adapter.
4. Implement fallback local compressors.
5. Implement retrieval store using SQLite + artifact store.
6. Implement compression stats.
7. Add compression into tool result path.
8. Add compression into conversation history path.
9. Add compression into web research path.
10. Add evals for answer preservation.

## Failure Modes

- Headroom unavailable.
- Compression corrupts syntax.
- Model cannot retrieve enough context.
- Retrieval store missing raw artifact.
- Sensitive data compressed before redaction.
- Marker collision.

## Tests

- Compress/retrieve round trip.
- Tool output compression.
- Code file compression preserves identifiers.
- JSON compression preserves schema keys.
- Retrieval tool returns original content.
- Disable compression for sensitive content.
- Over-compression triggers retry with lower compression.

