# Two bytes (0xff 0xfe) that are not valid UTF-8, then a clean line. The second line is the
# point: a reader that died on the first would never deliver it.
printf 'bad \377\376 bytes\n'
printf 'after\n'
