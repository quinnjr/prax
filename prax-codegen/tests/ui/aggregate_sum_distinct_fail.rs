// Fixture: _sum: { views: { distinct: true } } — distinct is only valid inside _count
// Expected diagnostic: "`distinct` is only valid inside `_count`, not `_sum`"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        _sum: { views: { distinct: true } },
    });
}
