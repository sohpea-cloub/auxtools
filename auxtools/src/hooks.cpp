#include <stdint.h>
#include "hooks.h"

using Runtime_Ptr = void(*)(char *pError);
using CallProc_Callback = void(*)(Value, uint32_t);
using CallProcById_Ptr = Value(*)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, CallProc_Callback, uint32_t);

using CallProcById_Hook_Ptr = Value(*)(Value, uint32_t, uint32_t, uint32_t, Value, Value*, uint32_t, uint32_t, uint32_t);

static const uint32_t PROC_TYPE_DISCARD_RESULT = 0x04;

extern "C" {
	Runtime_Ptr runtime_byond = nullptr;

	Runtime_Ptr runtime_original = nullptr;
	CallProcById_Ptr call_proc_by_id_original = nullptr;

	extern void (*dec_ref_count_byond)(Value);
}

extern "C" {
	bool runtime_intercepted = false;
}

#if defined(__i386__) && !defined(_WIN32)
extern "C" void call_keeping_registers(void* function, const uint32_t* words);
__asm__(
	".text\n"
	".globl call_keeping_registers\n"
	".hidden call_keeping_registers\n"
	".type call_keeping_registers, @function\n"
	"call_keeping_registers:\n"
	"	.cfi_startproc\n"
	"	pushl %ebp\n"
	"	.cfi_adjust_cfa_offset 4\n"
	"	.cfi_rel_offset ebp, 0\n"
	"	pushl %ebx\n"
	"	.cfi_adjust_cfa_offset 4\n"
	"	.cfi_rel_offset ebx, 0\n"
	"	pushl %esi\n"
	"	.cfi_adjust_cfa_offset 4\n"
	"	.cfi_rel_offset esi, 0\n"
	"	pushl %edi\n"
	"	.cfi_adjust_cfa_offset 4\n"
	"	.cfi_rel_offset edi, 0\n"
	"	movl 20(%esp), %eax\n"
	"	movl 24(%esp), %edx\n"
	"	subl $12, %esp\n"
	"	.cfi_adjust_cfa_offset 12\n"
	"	pushl 44(%edx)\n"
	"	pushl 40(%edx)\n"
	"	pushl 36(%edx)\n"
	"	pushl 32(%edx)\n"
	"	pushl 28(%edx)\n"
	"	pushl 24(%edx)\n"
	"	pushl 20(%edx)\n"
	"	pushl 16(%edx)\n"
	"	pushl 12(%edx)\n"
	"	pushl 8(%edx)\n"
	"	pushl 4(%edx)\n"
	"	pushl (%edx)\n"
	"	.cfi_adjust_cfa_offset 48\n"
	"	call *%eax\n"
	"	.cfi_adjust_cfa_offset -4\n"
	"	addl $56, %esp\n"
	"	.cfi_adjust_cfa_offset -56\n"
	"	popl %edi\n"
	"	.cfi_adjust_cfa_offset -4\n"
	"	.cfi_restore edi\n"
	"	popl %esi\n"
	"	.cfi_adjust_cfa_offset -4\n"
	"	.cfi_restore esi\n"
	"	popl %ebx\n"
	"	.cfi_adjust_cfa_offset -4\n"
	"	.cfi_restore ebx\n"
	"	popl %ebp\n"
	"	.cfi_adjust_cfa_offset -4\n"
	"	.cfi_restore ebp\n"
	"	ret\n"
	"	.cfi_endproc\n"
	".size call_keeping_registers, .-call_keeping_registers\n"
);
#endif

static Value call_original(
	Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t override_depth,
	Value src,
	Value* args,
	uint32_t args_count,
	CallProc_Callback callback,
	uint32_t callback_value
) {
#if defined(__i386__) && !defined(_WIN32)
	Value ret;
	const uint32_t words[12] = {
		(uint32_t)&ret, usr.type, usr.value, proc_type, proc_id, override_depth,
		src.type, src.value, (uint32_t)args, args_count, (uint32_t)callback, callback_value
	};
	call_keeping_registers((void*)call_proc_by_id_original, words);
	return ret;
#else
	return call_proc_by_id_original(usr, proc_type, proc_id, override_depth, src, args, args_count, callback, callback_value);
#endif
}

extern "C" void on_runtime(const char* pError);

extern "C" void runtime_hook(char* pError) {
	const char* pErrorCorrected = (pError != nullptr) ? pError : "<null>";
	if (runtime_intercepted) {
#ifdef USE_SJLJ
		longjmp(*current_jmp, 1);
#else
		throw AuxtoolsException(pErrorCorrected);
#endif
		return;
	}

	on_runtime(pErrorCorrected);
	return runtime_original(pError);
}

extern "C" uint8_t call_proc_by_id_hook(
	Value* ret,
	Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t override_depth,
	Value src,
	Value* args,
	uint32_t args_count,
	CallProc_Callback callback,
	uint32_t callback_value);

extern "C" Value call_proc_by_id_hook_trampoline(
	Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t override_depth,
	Value src,
	Value* args,
	uint32_t args_count,
	CallProc_Callback callback,
	uint32_t callback_value
) {
	Value ret;

	RuntimeContext runtime_scope(false);

	if (!call_proc_by_id_hook(&ret, usr, proc_type, proc_id, override_depth, src, args, args_count, callback, callback_value)) {
		return call_original(usr, proc_type, proc_id, override_depth, src, args, args_count, callback, callback_value);
	}
	clean(ret);

	if (callback != nullptr) {
		callback(ret, callback_value);
	}
	if (callback != nullptr || (proc_type & PROC_TYPE_DISCARD_RESULT)) {
		dec_ref_count_byond(ret);
		ret = Value{0, 0};
	}
	return ret;
}
