# Never writes, never exits, and cannot be stopped politely: the fixture for the timeout and
# cancellation paths.
#
# `trap '' TERM INT` is what makes it a "child that ignores a polite signal". Nothing in the
# app sends either one — the kill is SIGKILL on unix and TerminateProcess on Windows, and
# neither can be trapped — so the point is that the run ends anyway rather than waiting out
# the signal.
#
# Builtins only, and deliberately no `sleep`. A `sleep` here is a native binary, and Cygwin's
# `sh` cannot `exec` into one: it forks, the grandchild inherits these pipes, and it holds
# both readers open for its full lifetime after this shell is killed. That is an artefact of
# driving fixtures through `sh` — a real harness is spawned directly, with no shell in
# between — and it must not leak into what these tests measure.
trap '' TERM INT
while :; do :; done
