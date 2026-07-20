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

`define UVM_NONE   0
`define UVM_LOW   100
`define UVM_MEDIUM 200
`define UVM_HIGH  300
`define UVM_FULL  400
`define UVM_DEBUG 500
