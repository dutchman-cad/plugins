pub mod panel {
    wasmtime::component::bindgen!({
        path: "../wit", world: "panel-plugin",
        imports: { default: async }, exports: { default: async },
    });
}
pub mod command {
    wasmtime::component::bindgen!({
        path: "../wit", world: "command-plugin",
        imports: { default: async }, exports: { default: async },
        with: {
            "solidcraft:plugin/engine": super::panel::solidcraft::plugin::engine,
            "solidcraft:plugin/log": super::panel::solidcraft::plugin::log,
            "solidcraft:plugin/store": super::panel::solidcraft::plugin::store,
            "solidcraft:plugin/types": super::panel::solidcraft::plugin::types,
        },
    });
}
pub mod importer {
    wasmtime::component::bindgen!({
        path: "../wit", world: "importer-plugin",
        imports: { default: async }, exports: { default: async },
        with: {
            "solidcraft:plugin/log": super::panel::solidcraft::plugin::log,
            "solidcraft:plugin/types": super::panel::solidcraft::plugin::types,
        },
    });
}
