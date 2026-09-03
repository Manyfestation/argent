/**
 * @file Argent language grammar
 * @author Argent-lang developers
 * @license ISC
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

const PREC = {
  LOGICAL_OR: 1,
  LOGICAL_AND: 2,
  BIT_OR: 3,
  BIT_XOR: 4,
  BIT_AND: 5,
  EQUALITY: 6,
  COMPARISON: 7,
  TERM: 8,
  FACTOR: 9,
  UNARY: 10,
  POSTFIX: 11,
};

export default grammar({
  name: "argent",

  extras: ($) => [/\s/, $.comment],

  word: ($) => $.identifier,

  conflicts: ($) => [
    [$.assignment_statement, $.primary_expression],
    [$.block, $.object_literal],
    [$.primary_expression, $.base_type],
    [$.struct_destructure_assignment, $.typed_literal],
  ],

  rules: {
    source_file: ($) => repeat($._declaration),

    _declaration: ($) =>
      choice(
        $.import_declaration,
        $.constant_declaration,
        $.state_declaration,
        $.function_declaration,
        $.actor_enum_declaration,
        $.actor_declaration,
        $.app_declaration,
      ),

    import_declaration: ($) =>
      seq(
        "import",
        choice(
          $.string_literal,
          seq(
            choice(
              seq(
                "actor",
                field("subject", choice($.qualified_identifier, $.identifier)),
              ),
              seq("app", field("subject", $.identifier)),
            ),
            "from",
            field("path", $.string_literal),
          ),
        ),
        ";",
      ),

    constant_declaration: ($) =>
      seq(
        "const",
        field("type", $.type_name),
        field("name", $.identifier),
        "=",
        field("value", $.expression),
        ";",
      ),

    state_declaration: ($) =>
      seq(
        "state",
        field("name", $.identifier),
        optional(seq("expands", field("base", $.identifier))),
        "{",
        repeat(choice($.state_field_declaration, $.state_digest_declaration)),
        "}",
      ),

    state_field_declaration: ($) =>
      choice(
        seq("virtual", field("name", $.identifier), ";"),
        seq(
          field("type", $.type_name),
          field("name", $.identifier),
          ";",
        ),
      ),

    state_digest_declaration: ($) =>
      seq(
        field("name", $.identifier),
        ":",
        field("state", $.identifier),
        ";",
      ),

    function_declaration: ($) =>
      seq(
        "fn",
        field("name", $.identifier),
        $.parameter_list,
        optional(seq("->", field("return_type", $.type_name))),
        field("body", $.block),
      ),

    actor_enum_declaration: ($) =>
      seq(
        "actor",
        "enum",
        field("name", $.identifier),
        "{",
        repeat(seq($.actor_enum_variant, optional(choice(",", ";")))),
        "}",
      ),

    actor_enum_variant: ($) => field("name", $.identifier),

    actor_declaration: ($) =>
      seq(
        "actor",
        field("name", $.identifier),
        "owns",
        field("state", $.identifier),
        "{",
        repeat(choice($.function_declaration, $.entry_declaration, $.delegate_declaration)),
        "}",
      ),

    entry_declaration: ($) =>
      seq(
        "entry",
        field("name", $.identifier),
        $.parameter_list,
        repeat($._entry_clause),
        "emits",
        field("outputs", $.emit_specification),
        field("body", $.block),
      ),

    delegate_declaration: ($) =>
      seq(
        "delegate",
        field("name", $.identifier),
        $.parameter_list,
        repeat($._entry_clause),
        field("body", $.block),
      ),

    _entry_clause: ($) =>
      choice($.consumes_clause, $.observes_clause, $.spawns_clause),

    consumes_clause: ($) =>
      seq("consumes", "{", optional(commaSep($.actor_binding)), "}"),

    observes_clause: ($) =>
      seq(
        "observes",
        field("name", $.identifier),
        "by",
        field("covenant", $.expression),
        "{",
        repeat(choice($.inputs_section, $.outputs_section)),
        "}",
      ),

    spawns_clause: ($) =>
      seq(
        "spawns",
        field("name", $.identifier),
        "by",
        field("covenant", $.identifier),
        "{",
        $.outputs_section,
        "}",
      ),

    inputs_section: ($) =>
      seq(
        "inputs",
        "{",
        optional(commaSep(choice($.actor_binding, $.open_actor_binding))),
        "}",
      ),

    outputs_section: ($) =>
      seq("outputs", "{", optional(commaSep($.actor_binding)), "}"),

    actor_binding: ($) =>
      seq(
        field("name", $.identifier),
        ":",
        field("actor", $.actor_reference),
        optional($.cardinality),
      ),

    open_actor_binding: ($) =>
      seq(
        field("name", $.identifier),
        ":",
        "actor_type",
        "<",
        field("state", $.identifier),
        ">",
        "as",
        field("actor", $.identifier),
        optional($.cardinality),
      ),

    emit_specification: ($) =>
      choice(
        "none",
        seq("{", optional(commaSep($.emit_binding)), "}"),
        $.emit_binding,
      ),

    emit_binding: ($) =>
      seq(
        field("name", $.identifier),
        ":",
        field("actor", $.actor_union),
        optional($.cardinality),
      ),

    actor_union: ($) =>
      seq($.actor_reference, repeat(seq("|", $.actor_reference))),

    actor_reference: ($) =>
      choice(
        $.qualified_identifier,
        seq($.identifier, repeat1($.member_access)),
        $.identifier,
      ),

    cardinality: ($) =>
      seq(
        "[",
        field("minimum", $.cardinality_bound),
        "..=",
        field("maximum", $.cardinality_bound),
        "]",
      ),

    cardinality_bound: ($) => choice($.number_literal, $.identifier),

    app_declaration: ($) =>
      seq(
        "app",
        field("name", $.identifier),
        "{",
        repeat($.app_actor_declaration),
        "}",
      ),

    app_actor_declaration: ($) => seq("actor", field("name", $.identifier), ";"),

    parameter_list: ($) => seq("(", optional(commaSep($.parameter)), ")"),

    parameter: ($) =>
      seq(field("type", $.type_name), field("name", $.identifier)),

    block: ($) => seq("{", repeat($.statement), "}"),

    statement: ($) =>
      choice(
        $.variable_declaration,
        $.struct_destructure_assignment,
        $.tuple_assignment,
        $.assignment_statement,
        $.return_statement,
        $.if_statement,
        $.for_statement,
        $.require_statement,
        $.become_statement,
        $.validate_outputs_statement,
        $.expression_statement,
        $.block,
      ),

    variable_declaration: ($) =>
      seq(
        field("type", $.type_name),
        repeat("constant"),
        field("name", $.identifier),
        optional(seq("=", field("value", $.expression))),
        ";",
      ),

    struct_destructure_assignment: ($) =>
      seq(
        field("type", $.type_name),
        "{",
        optional(commaSep($.typed_field_binding)),
        "}",
        "=",
        field("value", $.expression),
        ";",
      ),

    typed_field_binding: ($) =>
      seq(
        field("field", $.identifier),
        ":",
        field("type", $.type_name),
        field("name", $.identifier),
      ),

    tuple_assignment: ($) =>
      seq(
        "(",
        commaSep1(seq($.type_name, $.identifier)),
        ")",
        "=",
        $.expression,
        ";",
      ),

    assignment_statement: ($) =>
      seq(
        field("left", choice($.identifier, $.postfix_expression)),
        "=",
        field("right", $.expression),
        ";",
      ),

    return_statement: ($) => seq("return", optional($.expression), ";"),

    if_statement: ($) =>
      prec.right(
        seq(
          "if",
          "(",
          field("condition", $.expression),
          ")",
          field("consequence", $.statement),
          optional(seq("else", field("alternative", $.statement))),
        ),
      ),

    for_statement: ($) =>
      seq(
        "for",
        "(",
        field("binding", $.identifier),
        ",",
        field("start", $.expression),
        ",",
        field("end", $.expression),
        ",",
        field("step", $.expression),
        ")",
        field("body", $.statement),
      ),

    require_statement: ($) =>
      seq(
        "require",
        "(",
        field("condition", $.expression),
        optional(seq(",", field("message", $.string_literal))),
        ")",
        ";",
      ),

    become_statement: ($) =>
      seq(
        "become",
        choice($.become_route, seq("{", optional(commaSep($.become_route)), "}")),
        optional(";"),
      ),

    validate_outputs_statement: ($) =>
      seq(
        "require",
        field("group", $.identifier),
        ".",
        "outputs",
        "become",
        "{",
        optional(commaSep($.become_route)),
        "}",
        optional(";"),
      ),

    become_route: ($) =>
      seq(
        field("output", $.identifier),
        "<-",
        field("successor", $.expression),
      ),

    expression_statement: ($) => seq($.expression, ";"),

    expression: ($) => $.logical_or_expression,

    logical_or_expression: ($) =>
      prec.left(
        PREC.LOGICAL_OR,
        seq($.logical_and_expression, repeat(seq("||", $.logical_and_expression))),
      ),

    logical_and_expression: ($) =>
      prec.left(
        PREC.LOGICAL_AND,
        seq($.bit_or_expression, repeat(seq("&&", $.bit_or_expression))),
      ),

    bit_or_expression: ($) =>
      prec.left(
        PREC.BIT_OR,
        seq($.bit_xor_expression, repeat(seq("|", $.bit_xor_expression))),
      ),

    bit_xor_expression: ($) =>
      prec.left(
        PREC.BIT_XOR,
        seq($.bit_and_expression, repeat(seq("^", $.bit_and_expression))),
      ),

    bit_and_expression: ($) =>
      prec.left(
        PREC.BIT_AND,
        seq($.equality_expression, repeat(seq("&", $.equality_expression))),
      ),

    equality_expression: ($) =>
      prec.left(
        PREC.EQUALITY,
        seq($.comparison_expression, repeat(seq(choice("==", "!="), $.comparison_expression))),
      ),

    comparison_expression: ($) =>
      prec.left(
        PREC.COMPARISON,
        seq($.additive_expression, repeat(seq(choice("<", "<=", ">", ">="), $.additive_expression))),
      ),

    additive_expression: ($) =>
      prec.left(
        PREC.TERM,
        seq($.multiplicative_expression, repeat(seq(choice("+", "-"), $.multiplicative_expression))),
      ),

    multiplicative_expression: ($) =>
      prec.left(
        PREC.FACTOR,
        seq($.unary_expression, repeat(seq(choice("*", "/", "%"), $.unary_expression))),
      ),

    unary_expression: ($) =>
      prec.right(PREC.UNARY, seq(repeat(choice("!", "-")), $.postfix_expression)),

    postfix_expression: ($) =>
      prec.left(PREC.POSTFIX, seq($.primary_expression, repeat($.postfix_operator))),

    postfix_operator: ($) =>
      choice(
        $.index_expression,
        $.member_access,
        $.call_suffix,
        $.as_cast,
      ),

    index_expression: ($) => seq("[", $.expression, "]"),

    member_access: ($) => seq(".", field("name", $.identifier)),

    call_suffix: ($) => $.argument_list,

    as_cast: ($) => prec.right(seq("as", $.type_name)),

    primary_expression: ($) =>
      choice(
        $.parenthesized_expression,
        $.instantiation_expression,
        $.call_expression,
        $.typed_literal,
        $.object_literal,
        $.qualified_identifier,
        $.identifier,
        $.literal,
      ),

    parenthesized_expression: ($) => seq("(", $.expression, ")"),

    instantiation_expression: ($) =>
      seq(
        "new",
        field("type", choice($.qualified_identifier, $.identifier)),
        $.argument_list,
      ),

    call_expression: ($) =>
      seq(field("function", $.type_name), $.argument_list),

    argument_list: ($) => seq("(", optional(commaSep($.expression)), ")"),

    typed_literal: ($) =>
      seq(
        field("type", $.type_name),
        "{",
        optional(commaSep(choice($.field_initializer, $.expression))),
        "}",
      ),

    object_literal: ($) =>
      seq("{", optional(commaSep($.field_initializer)), "}"),

    field_initializer: ($) =>
      seq(field("name", $.identifier), ":", field("value", $.expression)),

    qualified_identifier: ($) =>
      seq(field("namespace", $.identifier), "::", field("name", $.identifier)),

    type_name: ($) =>
      prec.right(
        seq(
          choice($.actor_type, $.base_type),
          repeat($.array_suffix),
        ),
      ),

    actor_type: ($) =>
      seq("actor_type", "<", field("state", $.identifier), ">"),

    base_type: ($) =>
      choice(
        "bool",
        "byte",
        "bytes",
        "cov_id",
        "datasig",
        "int",
        "pubkey",
        "sig",
        "string",
        "temporal",
        $.identifier,
      ),

    array_suffix: ($) => seq("[", optional($.array_bound), "]"),

    array_bound: ($) => choice("_", $.number_literal, $.identifier),

    literal: ($) =>
      choice(
        $.boolean_literal,
        $.hex_literal,
        $.number_literal,
        $.string_literal,
      ),

    boolean_literal: (_) => choice("true", "false"),

    hex_literal: (_) => token(/0[xX][0-9a-fA-F]*/),

    number_literal: (_) => token(/\d+(?:_\d+)*/),

    string_literal: (_) => token(/"([^"\\\n]|\\.)*"/),

    identifier: (_) => token(prec(-1, /[A-Za-z_][A-Za-z0-9_]*/)),

    comment: (_) =>
      token(choice(/\/\/[^\n]*/, /\/\*[^*]*\*+([^/*][^*]*\*+)*\//)),
  },
});

/**
 * @param {RuleOrLiteral} rule
 */
function commaSep(rule) {
  return seq(rule, repeat(seq(",", rule)), optional(","));
}

/**
 * @param {RuleOrLiteral} rule
 */
function commaSep1(rule) {
  return seq(rule, repeat1(seq(",", rule)), optional(","));
}
