pub(crate) struct Method {
    pub derive: &'static str,
    pub name: &'static str,
    pub labels: &'static [&'static str],
}

pub(crate) const METHODS: &[Method] = &[
    Method {
        derive: "Json",
        name: "to_json",
        labels: &["value"],
    },
    Method {
        derive: "Json",
        name: "from_json",
        labels: &["source"],
    },
    Method {
        derive: "Compare",
        name: "compare",
        labels: &["left", "right"],
    },
    Method {
        derive: "Display",
        name: "display",
        labels: &["value"],
    },
    Method {
        derive: "Key",
        name: "key",
        labels: &["value"],
    },
    Method {
        derive: "Validate",
        name: "validate",
        labels: &["value"],
    },
    Method {
        derive: "Binary",
        name: "to_binary",
        labels: &["value"],
    },
    Method {
        derive: "Binary",
        name: "from_binary",
        labels: &["bytes"],
    },
    Method {
        derive: "DatabaseRow",
        name: "from_row",
        labels: &["source"],
    },
    Method {
        derive: "Arguments",
        name: "from_arguments",
        labels: &["arguments"],
    },
];

pub(crate) fn named(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|method| method.name == name)
}
