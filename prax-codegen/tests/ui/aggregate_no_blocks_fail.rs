// Fixture: aggregate! with no aggregate block (_count/_sum/_avg/_min/_max)
// Expected diagnostic: "aggregate! requires at least one of _count, _sum, _avg, _min, _max"

fn main() {
    let client = ();
    let _ = prax_codegen::aggregate!(client.user, {
        where: { id: 1 },
    });
}
