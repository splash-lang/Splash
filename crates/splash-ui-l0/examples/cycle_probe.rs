//! Dispatch an event N times and print the state after each.
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let card = std::fs::read_to_string(&a[0]).expect("card");
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&a[1]).expect("data")).expect("json");
    let event = &a[2];
    let n: usize = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(3);
    let mut store = splash_ui_l0::InstanceStore::default();
    for i in 0..n {
        // Through the entry point the APP uses — the one that is handed the data
        // blob. `dispatch_with` passes an empty one, which is exactly the
        // difference this probe exists to see.
        splash_ui_l0::dispatch_reporting(&card, &mut store, "root", event, None, &data);
        let r = splash_ui_l0::realize_with_state(&card, &data, &store, Default::default());
        let dsl = splash_ui_l0::kit::lower(&r.root.expect("root"));
        let screen = if dsl.contains("l0_surface_map") && dsl.contains("follow") {
            "drive"
        } else {
            "plan"
        };
        println!("tap {} -> screen={screen}", i + 1);
    }
}
