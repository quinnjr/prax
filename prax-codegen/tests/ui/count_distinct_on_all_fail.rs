// Fixture: _count: { _all: { distinct: true } } — _all has no distinct form
// Expected diagnostic: "`_all` has no distinct form; use COUNT(*) via `_all: true`"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _count: { _all: { distinct: true } },
    });
}
