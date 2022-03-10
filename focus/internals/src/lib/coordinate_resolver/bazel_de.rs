//! Deserialization of Bazel query results (returned from `bazel query --output xml`).
//!
//! Check the documentation of
//! [`serde_xml_rs`](https://docs.rs/serde-xml-rs/latest/serde_xml_rs/) for
//! important information about the deserialization of XML.
//!
//! Some notes:
//!
//! - XML child elements are an ordered list.
//! - Unrecognized elements/attributes are ignored by `serde` by default unless
//! we enable `deny_unknown_fields`. However, unknown `enum` variants throw an
//! error when encountered, so we have to list them even if we're not going to
//! use them.
//! - Self-closing tags are omitted from the parse altogether, which is why
//! `Vec`s in this file are annotated with `#[serde(default)]`, or else a parse
//! error would be thrown.
//! - As a result of `#[serde(default)]` and the lack of `deny_unknown_fields`,
//! the nesting structure of `struct`s in this file is significant. It's
//! possible to get a successful parse but with no data if you're missing a
//! layer of nesting.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Query {
    #[serde(default, rename = "$value")]
    pub rules: Vec<QueryElement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum QueryElement {
    Rule(Rule),
    SourceFile {
        name: String,

        #[serde(default, rename = "$value")]
        body: (),
    },
    GeneratedFile {
        #[serde(default, rename = "$value")]
        body: (),
    },
    PackageGroup {
        #[serde(default, rename = "$value")]
        body: (),
    },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Rule {
    pub name: String,
    #[serde(default, rename = "$value")]
    pub elements: Vec<RuleElement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum RuleElement {
    Boolean {
        name: String,
        value: String,
    },
    String {
        name: String,
        value: String,
    },
    List {
        name: String,
        #[serde(default, rename = "$value")]
        values: Vec<Label>,
    },
    Dict {
        name: String,
    },
    Label(Label),
    VisibilityLabel {
        name: String,
    },
    RuleInput {
        name: String,
    },
    RuleOutput {
        name: String,
    },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Label {
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_bazel_query() -> anyhow::Result<()> {
        let payload = r#"
<?xml version="1.1" encoding="UTF-8" standalone="no"?>
<query version="2">
    <rule class="third_party_jvm_import" location="/home/project/BUILD:108:12" name="//:scala-collection-compat">
        <string name="name" value="scala-collection-compat"/>
        <list name="visibility">
            <label value="//visibility:public"/>
        </list>
        <string name="generator_name" value="scala-collection-compat"/>
        <string name="generator_function" value="jar_library"/>
        <string name="generator_location" value="/home/project/BUILD:108:12"/>
        <list name="deps">
            <label value="@maven//:_scala-collection-compat"/>
        </list>
        <rule-input name="@maven//:_scala-collection-compat"/>
    </rule>
    <rule class="alias" location="/home/project/3rdparty/jvm/com/fasterxml/jackson/BUILD:16:7" name="//3rdparty/jvm/com/fasterxml/jackson:jackson-module-scala">
        <string name="name" value="jackson-module-scala"/>
        <list name="visibility">
            <label value="//visibility:public"/>
        </list>
        <dict name="dummy-testing-dict" />
        <string name="generator_name" value="jackson-module-scala"/>
        <string name="generator_function" value="target"/>
        <string name="generator_location" value="3rdparty/jvm/com/fasterxml/jackson/BUILD:16:7"/>
        <label name="actual" value="//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala"/>
        <rule-input name="//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala"/>
    </rule>
    <source-file location="/private/var/tmp/user/edb2428c3d1a64c0af66dd62c2299134/external/zlib/trees.c:1:1" name="@zlib//:trees.c">
        <visibility-label name="//visibility:public"/>
    </source-file>
    <generated-file generating-rule="@zlib//:copy_public_headers" location="/private/var/tmp/user/edb2428c3d1a64c0af66dd62c2299134/external/zlib/BUILD.bazel:24:8" name="@zlib//:zlib/include/inftrees.h"/>
    <package-group location="/private/var/tmp/user/07c5cb75abc6d06390f255c356743280/external/bazel_tools/tools/build_defs/cc/whitelists/starlark_hdrs_check/BUILD:3:14" name="@bazel_tools//tools/build_defs/cc/whitelists/starlark_hdrs_check:loose_header_check_allowed_in_toolchain">
        <list name="includes"/>
        <list name="packages"/>
    </package-group>
</query>
        "#;

        let result: Query = serde_xml_rs::from_str(payload)
            .map_err(|e| anyhow::anyhow!("deserialization error: {:?}", e))?;
        insta::assert_debug_snapshot!(result, @r###"
        Query {
            rules: [
                Rule(
                    Rule {
                        name: "//:scala-collection-compat",
                        elements: [
                            String {
                                name: "name",
                                value: "scala-collection-compat",
                            },
                            List {
                                name: "visibility",
                                values: [],
                            },
                            String {
                                name: "generator_name",
                                value: "scala-collection-compat",
                            },
                            String {
                                name: "generator_function",
                                value: "jar_library",
                            },
                            String {
                                name: "generator_location",
                                value: "/home/project/BUILD:108:12",
                            },
                            List {
                                name: "deps",
                                values: [],
                            },
                            RuleInput {
                                name: "@maven//:_scala-collection-compat",
                            },
                        ],
                    },
                ),
                Rule(
                    Rule {
                        name: "//3rdparty/jvm/com/fasterxml/jackson:jackson-module-scala",
                        elements: [
                            String {
                                name: "name",
                                value: "jackson-module-scala",
                            },
                            List {
                                name: "visibility",
                                values: [],
                            },
                            Dict {
                                name: "dummy-testing-dict",
                            },
                            String {
                                name: "generator_name",
                                value: "jackson-module-scala",
                            },
                            String {
                                name: "generator_function",
                                value: "target",
                            },
                            String {
                                name: "generator_location",
                                value: "3rdparty/jvm/com/fasterxml/jackson/BUILD:16:7",
                            },
                            Label(
                                Label {
                                    value: "//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala",
                                },
                            ),
                            RuleInput {
                                name: "//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala",
                            },
                        ],
                    },
                ),
                SourceFile {
                    name: "@zlib//:trees.c",
                    body: (),
                },
                GeneratedFile {
                    body: (),
                },
                PackageGroup {
                    body: (),
                },
            ],
        }
        "###);

        Ok(())
    }
}
