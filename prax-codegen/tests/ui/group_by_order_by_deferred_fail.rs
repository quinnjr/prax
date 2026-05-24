// Fixture: group_by! with order_by: which is not yet implemented
// Expected diagnostic: "`order_by:` on group_by! is not yet implemented in phase 6"

fn main() {
    let client = ();
    let _ = prax_codegen::group_by!(client.user, {
        by: [id],
        _count: { _all: true },
        order_by: { _sum: { age: desc } },
    });
}
