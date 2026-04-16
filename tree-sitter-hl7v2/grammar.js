/**
 * @file parses hl7 v2 messages
 * @author Jesse Kruse <jesse.kruse@outlook.de>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

module.exports = grammar({
  name: "hl7v2",

  rules: {
    message: ($) =>
      seq(
        $.msh,
        repeat($.segment),
      ),

    msh: ($) =>
      seq(
        "MSH",
        $.field_separator,
        $.msh_controls,
        repeat(seq($.field_separator, optional($.field))),
        $.segment_separator,
      ),

    msh_controls: ($) =>
      seq(
        $.component_separator,
        $.repetition_separator,
        $.escape_character,
        $.subcomponent_separator,
      ),

    // Generic Segment (e.g., PID, OBR, ORC, etc.)
    segment: ($) =>
      seq(
        $.segment_name,
        repeat(seq($.field_separator, optional($.field))),
        $.segment_separator,
      ),

    segment_name: ($) => /[A-Z0-9]{3}/,

    // HL7 Separators
    field_separator: ($) => "|",
    component_separator: ($) => "^",
    repetition_separator: ($) => "~",
    escape_character: ($) => "\\",
    subcomponent_separator: ($) => "&",
    segment_separator: ($) => choice("\r", "\n", "\r\n"),

    // Field Handling
    field: ($) =>
      seq(
        $.repeat,
        repeat(seq($.repetition_separator, $.repeat)),
      ),

    repeat: ($) =>
      choice(
        $.component,
        seq(optional($.component), repeat1(seq($.component_separator, optional($.component)))),
      ),

    component: ($) =>
      seq(
        $.subcomponent,
        repeat(seq($.subcomponent_separator, optional($.subcomponent))),
      ),

    subcomponent: ($) => choice($.number, $.string),

    // HL7 Data Types
    number: ($) => /\d+/,
    string: ($) => /[^|^~\\&\r\n]+/, // Avoids breaking on separators
  },
});
