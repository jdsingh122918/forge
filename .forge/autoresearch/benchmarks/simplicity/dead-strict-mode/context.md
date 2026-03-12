# Permission Mode Configuration

`PermissionMode` controls the security level for forge pipeline execution:
- **Readonly**: Only read-only tools available
- **Standard**: Auto-approve iterations below a file-change threshold
- **Autonomous**: Auto-approve all iterations
- **Strict**: Documented as "requires manual approval every iteration"

However, `Strict` mode has no unique behavior. The `tools_for_permission_mode` function returns `None` for both Strict and Standard. The `should_auto_approve` function treats `Strict | Standard` identically. No gate check or UI exists for manual iteration approval.

The Strict variant adds surface area (display, parsing, documentation, tests) for behavior that is identical to Standard. It was removed and mapped to Standard with a deprecation warning during deserialization.
