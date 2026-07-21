// UVM Macros for Maria RTL Simulator
// Built-in UVM infrastructure handles get_type_name() etc. via engine dispatch.
// These macros provide source-level compatibility for `include "uvm_macros.svh"`.
// Unknown directives (`uvm_*) prefixed with backtick are skipped by preprocessor.

`define uvm_info(ID, MSG, VERBOSITY) \
  $display("UVM_INFO %s: %s", ID, MSG)

`define uvm_warning(ID, MSG) \
  $display("UVM_WARNING %s: %s", ID, MSG)

`define uvm_error(ID, MSG) \
  $display("UVM_ERROR %s: %s", ID, MSG)

`define uvm_fatal(ID, MSG) \
  begin $display("UVM_FATAL %s: %s", ID, MSG); $finish; end

`define uvm_component_utils(TYPE)

`define uvm_object_utils(TYPE)

`define uvm_sequence_utils(TYPE, SEQUENCER)

`define uvm_sequencer_utils(TYPE)

`define uvm_reg_block_utils(TYPE)

`define uvm_reg_utils(TYPE)

`define uvm_reg_field_utils(TYPE)

`define uvm_field_int(ARG, FLAG)

`define uvm_field_string(ARG, FLAG)

`define uvm_field_enum(ARG, FLAG)

`define uvm_field_object(ARG, FLAG)

`define uvm_field_array_int(ARG, FLAG)

`define uvm_field_queue_int(ARG, FLAG)

`define uvm_object_utils_begin(TYPE)

`define uvm_object_utils_end

`define uvm_component_utils_begin(TYPE)

`define uvm_component_utils_end

`define uvm_declare_p_sequencer(SEQUENCER)

`define uvm_update_sequence_lib_and_item(TYPE)

`define uvm_printer

// ── Sequence Macros ───────────────────────────────────────────────────────────
// Standard UVM sequence macros for creating, randomizing, and sending items.
// These rely on the engine's built-in UVM method dispatch (start_item/finish_item).

`define uvm_create(SEQ_OR_ITEM) \
  begin \
    SEQ_OR_ITEM = new(); \
    if (m_sequencer != null) begin \
      set_sequencer(m_sequencer); \
    end \
  end

`define uvm_send(SEQ_OR_ITEM) \
  begin \
    start_item(SEQ_OR_ITEM); \
    finish_item(SEQ_OR_ITEM); \
  end

`define uvm_do(SEQ_OR_ITEM) \
  begin \
    `uvm_create(SEQ_OR_ITEM) \
    if (!SEQ_OR_ITEM.randomize()) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    `uvm_send(SEQ_OR_ITEM) \
  end

`define uvm_do_with(SEQ_OR_ITEM, CONSTRAINTS) \
  begin \
    `uvm_create(SEQ_OR_ITEM) \
    if (!SEQ_OR_ITEM.randomize() with CONSTRAINTS) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    `uvm_send(SEQ_OR_ITEM) \
  end

`define uvm_rand_send(SEQ_OR_ITEM) \
  begin \
    if (!SEQ_OR_ITEM.randomize()) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    `uvm_send(SEQ_OR_ITEM) \
  end

`define uvm_rand_send_with(SEQ_OR_ITEM, CONSTRAINTS) \
  begin \
    if (!SEQ_OR_ITEM.randomize() with CONSTRAINTS) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    `uvm_send(SEQ_OR_ITEM) \
  end

`define uvm_do_pri(SEQ_OR_ITEM, PRIORITY) \
  begin \
    `uvm_create(SEQ_OR_ITEM) \
    if (!SEQ_OR_ITEM.randomize()) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    start_item(SEQ_OR_ITEM, PRIORITY); \
    finish_item(SEQ_OR_ITEM); \
  end

`define uvm_do_pri_with(SEQ_OR_ITEM, PRIORITY, CONSTRAINTS) \
  begin \
    `uvm_create(SEQ_OR_ITEM) \
    if (!SEQ_OR_ITEM.randomize() with CONSTRAINTS) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    start_item(SEQ_OR_ITEM, PRIORITY); \
    finish_item(SEQ_OR_ITEM); \
  end

`define uvm_do_on(SEQ_OR_ITEM, SEQUENCER) \
  begin \
    SEQ_OR_ITEM = new(); \
    set_sequencer(SEQUENCER); \
    if (!SEQ_OR_ITEM.randomize()) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    start_item(SEQ_OR_ITEM); \
    finish_item(SEQ_OR_ITEM); \
  end

`define uvm_do_on_with(SEQ_OR_ITEM, SEQUENCER, CONSTRAINTS) \
  begin \
    SEQ_OR_ITEM = new(); \
    set_sequencer(SEQUENCER); \
    if (!SEQ_OR_ITEM.randomize() with CONSTRAINTS) \
      `uvm_error("RAND", $sformatf("Randomization failed for %s", SEQ_OR_ITEM.get_type_name())) \
    start_item(SEQ_OR_ITEM); \
    finish_item(SEQ_OR_ITEM); \
  end

// ── Objection Macros ──────────────────────────────────────────────────────────
// End-of-test objection mechanism

`define uvm_raise_objection(SEQ_OR_ITEM) \
  begin \
    if (SEQ_OR_ITEM != null) begin \
      SEQ_OR_ITEM.raise_objection(); \
    end \
  end

`define uvm_drop_objection(SEQ_OR_ITEM) \
  begin \
    if (SEQ_OR_ITEM != null) begin \
      SEQ_OR_ITEM.drop_objection(); \
    end \
  end

// ── Verbosity Constants ──────────────────────────────────────────────────────

// ── OVM Compatibility Macros ────────────────────────────────────────────────
// OVM macros are aliases for equivalent UVM macros.
// OVM (Open Verification Methodology) is the predecessor to UVM.

`define ovm_info(ID, MSG, VERBOSITY) `uvm_info(ID, MSG, VERBOSITY)
`define ovm_warning(ID, MSG) `uvm_warning(ID, MSG)
`define ovm_error(ID, MSG) `uvm_error(ID, MSG)
`define ovm_fatal(ID, MSG) `uvm_fatal(ID, MSG)

`define ovm_component_utils(TYPE) `uvm_component_utils(TYPE)
`define ovm_object_utils(TYPE) `uvm_object_utils(TYPE)
`define ovm_sequence_item_utils(TYPE) `uvm_object_utils(TYPE)
`define ovm_sequence_utils(TYPE, SEQUENCER) `uvm_sequence_utils(TYPE, SEQUENCER)
`define ovm_sequencer_utils(TYPE) `uvm_sequencer_utils(TYPE)

`define ovm_report_info(ID, MSG, VERBOSITY) \
  $display("OVM_INFO %s: %s", ID, MSG)
`define ovm_report_warning(ID, MSG) \
  $display("OVM_WARNING %s: %s", ID, MSG)
`define ovm_report_error(ID, MSG) \
  $display("OVM_ERROR %s: %s", ID, MSG)
`define ovm_report_fatal(ID, MSG) \
  begin $display("OVM_FATAL %s: %s", ID, MSG); $finish; end
`define ovm_reg_block_utils(TYPE) `uvm_reg_block_utils(TYPE)
`define ovm_reg_utils(TYPE) `uvm_reg_utils(TYPE)
`define ovm_reg_field_utils(TYPE) `uvm_reg_field_utils(TYPE)

`define ovm_field_int(ARG, FLAG) `uvm_field_int(ARG, FLAG)
`define ovm_field_string(ARG, FLAG) `uvm_field_string(ARG, FLAG)
`define ovm_field_enum(ARG, FLAG) `uvm_field_enum(ARG, FLAG)
`define ovm_field_object(ARG, FLAG) `uvm_field_object(ARG, FLAG)
`define ovm_field_array_int(ARG, FLAG) `uvm_field_array_int(ARG, FLAG)
`define ovm_field_queue_int(ARG, FLAG) `uvm_field_queue_int(ARG, FLAG)

`define ovm_object_utils_begin(TYPE) `uvm_object_utils_begin(TYPE)
`define ovm_object_utils_end `uvm_object_utils_end
`define ovm_component_utils_begin(TYPE) `uvm_component_utils_begin(TYPE)
`define ovm_component_utils_end `uvm_component_utils_end

`define ovm_declare_p_sequencer(SEQUENCER) `uvm_declare_p_sequencer(SEQUENCER)
`define ovm_update_sequence_lib_and_item(TYPE) `uvm_update_sequence_lib_and_item(TYPE)
`define ovm_printer `uvm_printer

`define ovm_create(SEQ_OR_ITEM) `uvm_create(SEQ_OR_ITEM)
`define ovm_send(SEQ_OR_ITEM) `uvm_send(SEQ_OR_ITEM)
`define ovm_do(SEQ_OR_ITEM) `uvm_do(SEQ_OR_ITEM)
`define ovm_do_with(SEQ_OR_ITEM, CONSTRAINTS) `uvm_do_with(SEQ_OR_ITEM, CONSTRAINTS)
`define ovm_rand_send(SEQ_OR_ITEM) `uvm_rand_send(SEQ_OR_ITEM)
`define ovm_rand_send_with(SEQ_OR_ITEM, CONSTRAINTS) `uvm_rand_send_with(SEQ_OR_ITEM, CONSTRAINTS)
`define ovm_do_pri(SEQ_OR_ITEM, PRIORITY) `uvm_do_pri(SEQ_OR_ITEM, PRIORITY)
`define ovm_do_pri_with(SEQ_OR_ITEM, PRIORITY, CONSTRAINTS) `uvm_do_pri_with(SEQ_OR_ITEM, PRIORITY, CONSTRAINTS)
`define ovm_do_on(SEQ_OR_ITEM, SEQUENCER) `uvm_do_on(SEQ_OR_ITEM, SEQUENCER)
`define ovm_do_on_with(SEQ_OR_ITEM, SEQUENCER, CONSTRAINTS) `uvm_do_on_with(SEQ_OR_ITEM, SEQUENCER, CONSTRAINTS)

`define ovm_raise_objection(SEQ_OR_ITEM) `uvm_raise_objection(SEQ_OR_ITEM)
`define ovm_drop_objection(SEQ_OR_ITEM) `uvm_drop_objection(SEQ_OR_ITEM)

`define OVM_NONE   0
`define OVM_LOW   100
`define OVM_MEDIUM 200
`define OVM_HIGH  300
`define OVM_FULL  400
`define OVM_DEBUG 500

`define UVM_NONE   0
`define UVM_LOW   100
`define UVM_MEDIUM 200
`define UVM_HIGH  300
`define UVM_FULL  400
`define UVM_DEBUG 500
