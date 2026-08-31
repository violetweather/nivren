# Diagnostic catalog (org.nivren.diagnostic.v1)

The Edition 5 diagnostic contract gives every published diagnostic a stable
identifier. Identifiers are never reused or renumbered; new diagnostics
append. `NivError::code()` maps a diagnostic to its identifier, and the CLI
prints it as `error[NIV5001]: …`. Every entry follows the intent-first voice:
what the program **attempted**, what was **found**, and one concrete
**correction** (surfaced by `NivError::suggestion()` where applicable).

This first catalog covers the Edition 5 surface. Earlier diagnostics join the
catalog as they are rewritten to the three-part contract; a diagnostic without
an identifier is not yet published and may still be reworded.

| Identifier | Diagnostic | Triggering example |
| --- | --- | --- |
| NIV5001 | A loop exit (`stop`/`skip`) has no enclosing `repeat` or `each` loop | `stop` at top level |
| NIV5002 | A loop exit would cross a function, task, or `using` boundary | `each x in [1] { define f { skip } }` |
| NIV5003 | A `choose` does not cover every value; it needs the missing cases or an `otherwise` arm | `choose 5 { case 1 => "one" }` |
| NIV5004 | A `choose` arm is unreachable, including an `otherwise` after exhaustive case arms | `choose 5 { otherwise => 1 case 2 => 2 }` |
| NIV5005 | A case is matched by more than one unguarded arm | two `case Red` arms |
| NIV5006 | `or` pattern alternatives bind different names or types | `case Large carries amount or Small => …` |
| NIV5007 | A guard or text hole attempted to perform an effect; those positions stay pure | `case any when perform 1 => …` |
| NIV5008 | A float literal is not a safe selector | `choose 1.5 { case 1.5 => … }` |
| NIV5009 | A text hole holds a value with no canonical text form | `text "{[1, 2]}"` |
| NIV5010 | A text literal grew beyond the declared payload limit | 17 MiB of interpolation |
| NIV5011 | An effect or declaration needs a capability an active `promise never` renounces | `promise never Time` + `std.time.sleep(0.1)` |
| NIV5012 | A capability scope falls outside the promised `only within` boundaries | `needs FileRead within "path:./other"` under a promise for `"path:./data"` |
| NIV5013 | A promise names something outside the closed capability vocabulary | `promise never Magic` |
| NIV5014 | A sample breaks its rules: duplicate/overlong title, missing final expression for `shows`, or a failing execution | two samples titled `"a"` |
| NIV5015 | An untrusted module called into `std.native` or `std.host` | host call without `trusted "reason"` |
| NIV5016 | A generator failed: unknown name, wrong labels or arity, a non-declaration result, or the declaration cap | `expand missing` |
| NIV5017 | A refutable pattern appeared in a binding position | `keep Point holds { x set 0 } set p` |
| NIV5018 | A replay diverged from its recorded trace | replaying a different program |
| NIV5019 | A dependency change would alter recorded authority without review | an install after a dependency adds `Network` |
| NIV5020 | Unsigned arithmetic overflowed, or unsigned negation was attempted | `std.uint.max() + one` |
