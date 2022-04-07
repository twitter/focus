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
pub struct List {
    pub name: String,
    #[serde(default, rename = "$value")]
    pub values: Vec<RuleElement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
pub enum RuleElement {
    Boolean {
        name: Option<String>,
        value: Option<String>,
    },
    Int {
        name: Option<String>,
        value: Option<isize>,
    },
    String {
        name: Option<String>,
        value: Option<String>,
    },
    List(List),
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
    Output {
        name: Option<String>,
    },
    Tristate {
        name: String,
    },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Label {
    pub name: Option<String>,
    pub value: Option<String>,
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
        <list name="tags">
            <string value="bazel-compatible"/>
        </list>
        <list name="visibility">
            <label value="//visibility:public"/>
        </list>
        <dict name="dummy-testing-dict" />
        <string name="generator_name" value="jackson-module-scala"/>
        <string name="generator_function" value="target"/>
        <string name="generator_location" value="3rdparty/jvm/com/fasterxml/jackson/BUILD:16:7"/>
        <label name="actual" value="//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala"/>
        <label value="//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala"/>
        <rule-input name="//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala"/>
        <output name="dummy-output" value="//foo/bar:baz"/>
        <int name="dummy-int" value="10"/>
        <tristate name="dummy-tristate" value="-1"/>
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
                                name: Some(
                                    "name",
                                ),
                                value: Some(
                                    "scala-collection-compat",
                                ),
                            },
                            List(
                                List {
                                    name: "visibility",
                                    values: [
                                        Label(
                                            Label {
                                                name: None,
                                                value: Some(
                                                    "//visibility:public",
                                                ),
                                            },
                                        ),
                                    ],
                                },
                            ),
                            String {
                                name: Some(
                                    "generator_name",
                                ),
                                value: Some(
                                    "scala-collection-compat",
                                ),
                            },
                            String {
                                name: Some(
                                    "generator_function",
                                ),
                                value: Some(
                                    "jar_library",
                                ),
                            },
                            String {
                                name: Some(
                                    "generator_location",
                                ),
                                value: Some(
                                    "/home/project/BUILD:108:12",
                                ),
                            },
                            List(
                                List {
                                    name: "deps",
                                    values: [
                                        Label(
                                            Label {
                                                name: None,
                                                value: Some(
                                                    "@maven//:_scala-collection-compat",
                                                ),
                                            },
                                        ),
                                    ],
                                },
                            ),
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
                                name: Some(
                                    "name",
                                ),
                                value: Some(
                                    "jackson-module-scala",
                                ),
                            },
                            List(
                                List {
                                    name: "tags",
                                    values: [
                                        String {
                                            name: None,
                                            value: Some(
                                                "bazel-compatible",
                                            ),
                                        },
                                    ],
                                },
                            ),
                            List(
                                List {
                                    name: "visibility",
                                    values: [
                                        Label(
                                            Label {
                                                name: None,
                                                value: Some(
                                                    "//visibility:public",
                                                ),
                                            },
                                        ),
                                    ],
                                },
                            ),
                            Dict {
                                name: "dummy-testing-dict",
                            },
                            String {
                                name: Some(
                                    "generator_name",
                                ),
                                value: Some(
                                    "jackson-module-scala",
                                ),
                            },
                            String {
                                name: Some(
                                    "generator_function",
                                ),
                                value: Some(
                                    "target",
                                ),
                            },
                            String {
                                name: Some(
                                    "generator_location",
                                ),
                                value: Some(
                                    "3rdparty/jvm/com/fasterxml/jackson/BUILD:16:7",
                                ),
                            },
                            Label(
                                Label {
                                    name: Some(
                                        "actual",
                                    ),
                                    value: Some(
                                        "//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala",
                                    ),
                                },
                            ),
                            Label(
                                Label {
                                    name: None,
                                    value: Some(
                                        "//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala",
                                    ),
                                },
                            ),
                            RuleInput {
                                name: "//3rdparty/jvm/com/fasterxml/jackson/module:jackson-module-scala",
                            },
                            Output {
                                name: Some(
                                    "dummy-output",
                                ),
                            },
                            Int {
                                name: Some(
                                    "dummy-int",
                                ),
                                value: Some(
                                    10,
                                ),
                            },
                            Tristate {
                                name: "dummy-tristate",
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

    #[test]
    fn test_deserialize_bazel_query2() -> anyhow::Result<()> {
        let payload = "<?xml version=\"1.1\" encoding=\"UTF-8\" standalone =\"no\"?>
        <query version=\"2\">
            <source-file location=\"/private/var/folders/gn/gdp9z_g968b9nx7c9lvgy8y00000gp/T/.tmpBC1AHJ/repo_ebfc2762-2b78-484f-92f6-03ed35e38249/macro/BUILD:1:1\" name=\"//macro:BUILD\" package_contains_errors=\"false\">
                <visibility-label name=\"//visibility:private\"/>
            </source-file>
            <source-file location=\"/private/var/folders/gn/gdp9z_g968b9nx7c9lvgy8y00000gp/T/.tmpBC1AHJ/repo_ebfc2762-2b78-484f-92f6-03ed35e38249/package1/BUILD:1:1\" name=\"//package1:BUILD\" package_contains_errors=\"false\">
                <load name=\"//macro:macro.bzl\"/>
                <load name=\"//macro:macro_inner.bzl\"/>
                <visibility-label name=\"//visibility:private\"/>
            </source-file>
            <rule class=\"genrule\" location=\"/private/var/folders/gn/gdp9z_g968b9nx7c9lvgy8y00000gp/T/.tmpBC1AHJ/repo_ebfc2762-2b78-484f-92f6-03ed35e38249/package1/BUILD:2:9\" name=\"//package1:foo\">
                <string name=\"name\" value=\"foo\"/>
                <list name=\"tags\">
                    <string value=\"bazel-compatible\"/>
                </list>
                <string name=\"generator_name\" value=\"foo\"/>
                <string name=\"generator_function\" value=\"my_macro\"/>
                <string name=\"generator_location\" value=\"package1/BUILD:2:9\"/>
                <list name=\"srcs\">
                    <label value=\"//package2:contents\"/>
                </list>
                <list name=\"outs\">
                    <output value=\"//package1:out.txt\"/>
                </list>
                <string name=\"cmd\" value=\"cp $(SRCS) $@\"/>
                <rule-input name=\"//package2:contents\"/>
                <rule-input name=\"@bazel_tools//tools/genrule:genrule-setup.sh\"/>
                <rule-output name=\"//package1:out.txt\"/>
            </rule>
            <generated-file generating-rule=\"//package1:foo\" location=\"/private/var/folders/gn/gdp9z_g968b9nx7c9lvgy8y00000gp/T/.tmpBC1AHJ/repo_ebfc2762-2b78-484f-92f6-03ed35e38249/package1/BUILD:2:9\" name=\"//package1:out.txt\"/>
            <source-file location=\"/private/var/folders/gn/gdp9z_g968b9nx7c9lvgy8y00000gp/T/.tmpBC1AHJ/repo_ebfc2762-2b78-484f-92f6-03ed35e38249/package2/BUILD:1:1\" name=\"//package2:BUILD\" package_contains_errors=\"false\">
                <visibility-label name=\"//visibility:private\"/>
            </source-file>
            <source-file location=\"/private/var/folders/gn/gdp9z_g968b9nx7c9lvgy8y00000gp/T/.tmpBC1AHJ/repo_ebfc2762-2b78-484f-92f6-03ed35e38249/package2/contents:1:1\" name=\"//package2:contents\">
                <visibility-label name=\"//visibility:public\"/>
            </source-file>
        </query>
        ";
        let result: Query = serde_xml_rs::from_str(payload)
            .map_err(|e| anyhow::anyhow!("deserialization error: {:?}", e))?;
        insta::assert_debug_snapshot!(result, @r###"
        Query {
            rules: [
                SourceFile {
                    name: "//macro:BUILD",
                    body: (),
                },
                SourceFile {
                    name: "//package1:BUILD",
                    body: (),
                },
                Rule(
                    Rule {
                        name: "//package1:foo",
                        elements: [
                            String {
                                name: Some(
                                    "name",
                                ),
                                value: Some(
                                    "foo",
                                ),
                            },
                            List(
                                List {
                                    name: "tags",
                                    values: [
                                        String {
                                            name: None,
                                            value: Some(
                                                "bazel-compatible",
                                            ),
                                        },
                                    ],
                                },
                            ),
                            String {
                                name: Some(
                                    "generator_name",
                                ),
                                value: Some(
                                    "foo",
                                ),
                            },
                            String {
                                name: Some(
                                    "generator_function",
                                ),
                                value: Some(
                                    "my_macro",
                                ),
                            },
                            String {
                                name: Some(
                                    "generator_location",
                                ),
                                value: Some(
                                    "package1/BUILD:2:9",
                                ),
                            },
                            List(
                                List {
                                    name: "srcs",
                                    values: [
                                        Label(
                                            Label {
                                                name: None,
                                                value: Some(
                                                    "//package2:contents",
                                                ),
                                            },
                                        ),
                                    ],
                                },
                            ),
                            List(
                                List {
                                    name: "outs",
                                    values: [
                                        Output {
                                            name: None,
                                        },
                                    ],
                                },
                            ),
                            String {
                                name: Some(
                                    "cmd",
                                ),
                                value: Some(
                                    "cp $(SRCS) $@",
                                ),
                            },
                            RuleInput {
                                name: "//package2:contents",
                            },
                            RuleInput {
                                name: "@bazel_tools//tools/genrule:genrule-setup.sh",
                            },
                            RuleOutput {
                                name: "//package1:out.txt",
                            },
                        ],
                    },
                ),
                GeneratedFile {
                    body: (),
                },
                SourceFile {
                    name: "//package2:BUILD",
                    body: (),
                },
                SourceFile {
                    name: "//package2:contents",
                    body: (),
                },
            ],
        }
        "###);
        Ok(())
    }
}
