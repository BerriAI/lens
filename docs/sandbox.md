# Trace analysis sandbox

The main agent and bots use `run_python`; investigations use `python`. Both run ordinary CPython with the standard library in a fresh child process inside the existing worker container. It receives the selected evidence as `data` over stdin and has its own temporary working directory. It creates no additional container or service. Read and search tools remain available independently of Python

The native worker image builds a syscall policy with libseccomp and includes the full `setpriv` launcher. Each child starts with no inherited worker secrets or open worker files, isolated Python startup, Landlock filesystem restrictions and a default-deny seccomp filter. It can read the Python runtime and its own scratch files. Worker source, installed worker packages, other jobs' files and `/proc` contents are unavailable. Network sockets, child processes, cross-process memory operations, signals to other processes and filesystem metadata mutation are denied, including calls made through `ctypes`. Some metadata inspection, such as `stat`, `access` and `readlink` of known paths, remains possible

Python execution requires a native Linux worker with Landlock ABI 3 or later and seccomp filtering. Build the image for the host architecture. Missing policy files, an incompatible kernel, or an unsupported host such as a macOS source worker returns a clear tool error. There is no unrestricted execution fallback. Keep the container's non-root user, dropped capabilities, no-new-privileges setting, read-only root and writable temporary mount

The worker permits two Python children at once across all agents and investigations. Queued calls consume no child process or scratch directory; cancelling a queued call does not start it. Model, read and search concurrency are separate

| Per-call resource | Default |
| --- | --- |
| Elapsed execution time | 60 seconds |
| CPU time | 30 seconds |
| Process address space | 512 MiB |
| Captured stdout or stderr | 4 MiB per stream |
| Individual scratch file size | 16 MiB |
| Monitored scratch storage | 64 MiB |
| Monitored scratch entries | 2,048 |
| Scratch directory depth | 128 |
| Open file descriptors | 64 |

For `run_python`, supply `sql` to load a scope-checked read-only query result as `data`, or supply a JSON `data` value directly. Print the result, then use `publish_artifact` with inline content for charts, CSV or other attachments. Scratch files do not persist between calls. The sandbox does not install packages or call Lens tools

Investigation evidence is streamed from stored trace pages into the confined child without building another complete selection in worker memory. The child decodes the selected data under its memory limit before running the code. The execution wall clock includes input delivery; trace fetches keep their HTTP timeouts and remain cancellable. CPU, address-space and file-size limits apply during input decoding as well as computation. Scratch usage is monitored every 50 milliseconds, so a call can temporarily overshoot its scratch allowance. The worker's shared temporary mount supplies the hard aggregate storage ceiling, 1 GiB by default. Accounting includes unlinked open files and files retained only by memory mappings. A mapped scratch inode without an open descriptor or directory entry is conservatively charged at the individual file-size limit, which may overcount small files. Cancellation and limit failures kill and reap the child before removing its scratch directory

Results include `stdout`, `stderr`, `exit_code`, `error` and `output_complete`. Nonzero interpreter exits, confinement failures and resource failures set `error` and `output_complete=false`. Available traceback output is retained. An output-size failure retains the bounded output collected before the limit; the agent can narrow its computation and retry. A successful result retains all captured output without truncation

This is a process boundary sharing the worker's Linux kernel. The checked-in smoke test verifies useful Python operations, filesystem and process restrictions, raw syscall attempts, resource failures, mapping accounting, cleanup and cancellation in the actual image. Run it on the deployment's native architecture and kernel:

```bash
docker build --target smoke --build-arg LENS_VERSION=lens-python-test \
  -f deploy/runtime/Dockerfile -t lens-worker:smoke .
docker run --rm --read-only --cap-drop ALL \
  --security-opt no-new-privileges --network none --cpus 2 --memory 1g --pids-limit 32 \
  --tmpfs /tmp:rw,noexec,nosuid,size=256m lens-worker:smoke
```

The smoke target runs the Rust sandbox integration tests. The production image contains neither Cargo nor the test executable
