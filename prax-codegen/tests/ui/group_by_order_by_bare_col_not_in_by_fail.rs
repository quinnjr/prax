// Fixture: order_by bare column `region` is not in the `by:` list
// Expected diagnostic: "order by `region` requires `region` in `by:`"

fn main() {
    let client = ();
    let _ = prax_codegen::group_by!(client.user, {
        by: [team_id],
        _count: { _all: true },
        order_by: { region: desc },
    });
}
