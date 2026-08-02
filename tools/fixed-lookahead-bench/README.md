# Fixed-lookahead differential validation + benchmark

Reproduces the issue #150 acceptance evidence for `--fixed-lookahead` on the
three target grammars from `antlr/grammars-v4` (Thrift, EDN, Rego):

1. generates a **baseline** and a **`--fixed-lookahead`** parser per grammar,
2. parses each grammar's full example corpus with both — valid **and**
   invalid inputs (Rego ships 25 exact-`.errors` recovery fixtures) — and
   fails unless parse trees *and* error output are byte-identical,
3. runs a same-machine interleaved A/B benchmark (total min-parse time,
   per-file geomean speedup, cold first-parse time).

```bash
tools/fixed-lookahead-bench/run.sh                      # clones grammars-v4 (sparse) itself
tools/fixed-lookahead-bench/run.sh --grammars-v4 DIR    # reuse an existing checkout
```

Work products land under `target/fixed-lookahead-bench/`; the generated
`decisions.json` next to each parser lists every decision's tier
(`ll1` / `fixed` / `adaptive` + reason) and whether it `canDefer` to adaptive
prediction.

Reference numbers (Apple Silicon, 2026-07): Thrift +12% parse & fastest
cold-start (0 adaptive decisions at `--fixed-lookahead 2`), EDN +5%, Rego
parity-neutral with 1955/1955 files byte-identical.
