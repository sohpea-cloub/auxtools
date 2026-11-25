.intel_syntax noprefix
.global execute_instruction_hook
.global execute_instruction_hook_edi
.extern execute_instruction_original
.extern handle_instruction

execute_instruction_hook:
  SUB ESP, 0x04
  PUSH ECX
  PUSH EDX
  PUSH ESI
  call handle_instruction
  MOV ESI, EAX
  ADD ESP, 0x04
  POP EDX
  POP ECX
  ADD ESP, 0x04

  MOV EAX, execute_instruction_original
  JMP EAX

execute_instruction_hook_edi:
  SUB ESP, 0x04
  PUSH ECX
  PUSH EDX
  PUSH EDI
  call handle_instruction
  MOV EDI, EAX
  ADD ESP, 0x04
  POP EDX
  POP ECX
  ADD ESP, 0x04

  MOV EAX, execute_instruction_original
  JMP EAX
