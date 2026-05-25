// Fixture: order_by references _sum: { views } but no _sum block is present
// Expected diagnostic: "order by `_sum.views` requires a matching `_sum: { views }` block"

fn main() {
    let client = ();
    let _ = prax_codegen::group_by!(client.user, {
        by: [team_id],
        _count: { _all: true },
        order_by: { _sum: { views: desc } },
    });
}
