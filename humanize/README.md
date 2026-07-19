# humanize

Display a byte size in human-readable form using binary units (KiB, MiB, GiB, …).

## Usage

```bash
humanize <size>
```

`<size>` is a number of bytes.

## Examples

```bash
$ humanize 1024
1KiB

$ humanize 1536
1.5KiB

$ humanize 1234567
1.18MiB

$ humanize 1073741824
1GiB
```

Values below 1024 are shown in bytes:

```bash
$ humanize 512
512B
```

Mantissas keep up to two decimal places, with trailing zeros trimmed, so exact
powers of 1024 render cleanly (e.g. `1MiB` rather than `1.00MiB`).
