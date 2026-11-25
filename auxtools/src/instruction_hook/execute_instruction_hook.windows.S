.intel_syntax noprefix
.global _execute_instruction_hook
.extern _execute_instruction_original
.extern _handle_instruction

_execute_instruction_hook:
  PUSH EAX
  PUSH ECX
  PUSH EDI
  call _handle_instruction
  MOV EDI, EAX
  ADD ESP, 0x04
  POP ECX
  POP EAX

  MOV EAX, _execute_instruction_original
  JMP EAX
