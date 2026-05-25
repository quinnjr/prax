// Fixture: _count: { email: { distinct: false } } — block value must be `true` or `{ distinct: true }`
// Expected diagnostic: "value for `_count.email` must be `true` or `{ distinct: true }`"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _count: { email: { distinct: false } },
    });
}
