# Threat Model Template

## Module

`<module name>`

## Assets

- Source code
- Secrets
- User data
- Provider keys
- Filesystem
- Network access

## Trust Boundaries

- User input
- Model output
- Tool input
- Tool output
- MCP connector
- Web content
- External process

## Threats

| Threat | Impact | Likelihood | Mitigation |
|---|---|---|---|
| Prompt injection |  |  |  |
| Secret exfiltration |  |  |  |
| Unsafe command |  |  |  |
| Path traversal |  |  |  |
| Infinite loop/cost exhaustion |  |  |  |
| Over-compression loss |  |  |  |

## Required Controls

- [ ] Permission gate
- [ ] Redaction
- [ ] Sandbox
- [ ] Audit log
- [ ] Budget
- [ ] Verification

