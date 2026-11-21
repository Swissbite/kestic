# Strict addendum

Appended to a review prompt when all four reviews of a change with more than 400 changed lines
reported nothing.

---

Four reviews of this change found nothing. Changes of this size almost always contain defects,
so assume this one does. Do not answer «no findings». Instead:

1. Name the **three weakest points** of the change – the places where a defect is most likely –
   and for each show concretely why it holds or how it fails (code path, input, result).
2. For every AC, name the test line that would fail if the feature were removed. If there is
   none, that is `must-fix`.
3. Check every new or changed restic invocation against `docs/spec/restic-integration.md`
   flag by flag.

Use the same verdict format.
