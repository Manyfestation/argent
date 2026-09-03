(comment) @comment

(string_literal) @string

[
  (number_literal)
  (hex_literal)
] @number

(boolean_literal) @boolean

(state_declaration
  name: (identifier) @type)

(state_declaration
  base: (identifier) @type)

(actor_declaration
  name: (identifier) @type)

(actor_declaration
  state: (identifier) @type)

(actor_enum_declaration
  name: (identifier) @type)

(actor_enum_variant
  name: (identifier) @constant)

(app_declaration
  name: (identifier) @namespace)

(app_actor_declaration
  name: (identifier) @type)

(function_declaration
  name: (identifier) @function)

[
  (entry_declaration
    name: (identifier) @function)
  (delegate_declaration
    name: (identifier) @function)
]

(constant_declaration
  name: (identifier) @constant)

(state_field_declaration
  name: (identifier) @property)

(state_digest_declaration
  name: (identifier) @property
  state: (identifier) @type)

(parameter
  name: (identifier) @variable.parameter)

(variable_declaration
  name: (identifier) @variable)

(typed_field_binding
  field: (identifier) @property
  name: (identifier) @variable)

[
  (actor_binding
    name: (identifier) @variable)
  (open_actor_binding
    name: (identifier) @variable)
  (emit_binding
    name: (identifier) @variable)
]

(open_actor_binding
  state: (identifier) @type
  actor: (identifier) @variable)

(become_route
  output: (identifier) @variable)

(field_initializer
  name: (identifier) @property)

(member_access
  name: (identifier) @property)

(postfix_expression
  (primary_expression)
  (postfix_operator
    (member_access
      name: (identifier) @function.method))
  .
  (postfix_operator
    (call_suffix)))

(call_expression
  function: (type_name
    (base_type
      (identifier) @function)))

(instantiation_expression
  type: (identifier) @type)

(instantiation_expression
  type: (qualified_identifier
    namespace: (identifier) @namespace
    name: (identifier) @type))

(qualified_identifier
  namespace: (identifier) @namespace
  name: (identifier) @type)

(type_name
  (base_type
    (identifier) @type))

(actor_type
  "actor_type" @type.builtin
  state: (identifier) @type)

[
  "bool"
  "byte"
  "bytes"
  "cov_id"
  "datasig"
  "int"
  "pubkey"
  "sig"
  "string"
  "temporal"
] @type.builtin

((identifier) @variable.special
  (#match? @variable.special "^(self|this|tx)$"))

((identifier) @function.builtin
  (#match? @function.builtin "^(Op[A-Za-z0-9_]+|ScriptPubKeyP2PK|ScriptPubKeyP2SH|ScriptPubKeyP2SHFromRedeemScript|blake2b|blake2bWithKey|blake3|blake3WithKey|checkMsgSig|checkMsgSigEcdsa|checkSig|checkSigEcdsa|co_spent|digest|length|require|sha256|signed|state|templateHash|unrestricted|unsigned)$"))

[
  "actor"
  "app"
  "as"
  "become"
  "by"
  "consumes"
  "const"
  "constant"
  "delegate"
  "else"
  "emits"
  "entry"
  "enum"
  "expands"
  "fn"
  "for"
  "from"
  "if"
  "import"
  "inputs"
  "new"
  "none"
  "observes"
  "outputs"
  "owns"
  "return"
  "spawns"
  "state"
  "virtual"
] @keyword

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ":"
  "::"
  ","
  "."
  ";"
] @punctuation.delimiter

[
  "!"
  "!="
  "%"
  "&"
  "&&"
  "*"
  "+"
  "-"
  "->"
  "/"
  "<"
  "<-"
  "<="
  "="
  "=="
  ">"
  ">="
  "^"
  "|"
  "||"
  "..="
] @operator
