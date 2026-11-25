.MODEL FLAT, C
.CODE

EXTERN execute_instruction_original : PTR
handle_instruction PROTO, opcode: DWORD

execute_instruction_hook PROC PUBLIC
  PUSH EAX
  PUSH ECX
  INVOKE handle_instruction, EDI
  MOV EDI, EAX
  POP ECX
  POP EAX

  MOV EAX, execute_instruction_original
  JMP EAX
execute_instruction_hook ENDP

END
