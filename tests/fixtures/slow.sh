# A producer that is slower than a tick: the first line is available immediately, the second
# only after a pause. A drain that waited for the producer would show both at once.
printf 'first\n'
sleep 0.3
printf 'second\n'
