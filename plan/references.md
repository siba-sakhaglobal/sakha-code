# References

Research captured on July 10, 2026.

## Headroom

- GitHub: `https://github.com/headroomlabs-ai/headroom`
- Notes used:
  - Headroom describes itself as compressing tool outputs, logs, RAG chunks, files, and conversation history before LLM input.
  - It supports library, proxy, agent wrap, MCP tools, cross-agent memory, and reversible CCR retrieval.
  - Its architecture includes CacheAligner, ContentRouter, SmartCrusher, CodeCompressor, Kompress-v2-base, and CCR.
- External article: `https://www.theregister.com/ai-ml/2026/05/31/netflix-wiz-creates-app-to-slash-ai-bills-then-open-sources-it/5248702`
  - Useful notes: CacheAligner, AST/JSON/DOM compressors, feedback loop for over/under-compression, CCR storage in Redis or SQLite.

## Loop Engineering

- Addy Osmani: `https://addyosmani.com/blog/loop-engineering/`
  - Useful model: scheduled automations, worktrees, skills, plugins/connectors, sub-agents, durable memory.
- LangChain: `https://www.langchain.com/blog/the-art-of-loop-engineering`
  - Useful model: agent loop, verification loop, event-driven loop, human feedback, long-term improvement loops.
- Long-running agent harnesses: `https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents`
  - Useful model: initializer agent, incremental coding sessions, handoff artifacts, context-window bridging.
- Infinite agentic loops: `https://arxiv.org/abs/2607.01641`
  - Useful risk: unbounded model/tool/workflow loops can exhaust cost, grow context, and repeat side effects.
- LongSeeker / Context-ReAct: `https://arxiv.org/abs/2605.05191`
  - Useful operations: Skip, Compress, Rollback, Snippet, Delete for elastic context orchestration.
- LOOP Skill Engine: `https://arxiv.org/abs/2605.14237`
  - Useful idea: record successful task trajectory, extract deterministic replay skill, reduce repetitive token use.

## Platform Choices

- Rust: `https://www.rust-lang.org/`
- Tokio: `https://tokio.rs/`
- Tauri: `https://v2.tauri.app/`
- Go alternative: `https://go.dev/`
- MCP SDK overview: `https://modelcontextprotocol.io/docs/sdk`

