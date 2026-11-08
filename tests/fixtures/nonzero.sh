# Both streams written, then exit non-zero: the case that must report the status and keep
# the stderr line that explains it.
printf 'out one\n'
printf 'out two\n'
printf 'err one\n' >&2
exit 3
