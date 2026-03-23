# Security Policy

## Threat Model

`oxFileproc` is a powerful configuration processing library. Users should be aware of the following security boundaries:

### 1. Arbitrary File Read
**Risk**: High (if handling untrusted input).
**Description**: The `include`, `merge`, and `merge_recursive` directives allow reading files from the filesystem. The library **does not** enforce a sandbox or chroot by default. If you process a file that contains `include: "/etc/passwd"`, the library will attempt to read it.
**Mitigation**: 
- Do not process untrusted configuration files.
- Run the process in a sandboxed environment (container, restricted user) if managing untrusted input.

### 2. Denial of Service (DoS)
**Risk**: Medium.
**Description**: Recursive includes and variable expansion can consume resources.
**Mitigation**:
- **Recursion Depth**: `process_file` enforces a `max_depth` parameter (default 5 can be set by caller).
- **Cycle Detection**: The library tracks canonical paths of visited files and returns an error if a cycle is detected.

### 3. Variable Injection
**Risk**: Low.
**Description**: Currently, `${{VAR}}` substitution relies on a provided `Context` (in `substitutor.rs` logic) or environment depending on implementation.
**Mitigation**: The library only expands variables explicitly provided or available.

## Reporting Vulnerabilities
Please report vulnerabilities to the maintainer ensuring a responsible disclosure window.
