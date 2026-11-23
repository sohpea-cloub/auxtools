/datum
	var/__auxtools_weakref_id

/proc/auxtools_test_dll()
	. = world.GetConfig("env", "AUXTEST_DLL")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtest_out(msg)
	call_ext(auxtools_test_dll(), "auxtest_out")(msg)

/proc/auxtest_inc_counter()
	CRASH()

/proc/auxtest_failed_init(auxtest_dll)
	var/first = call_ext(auxtest_dll, "auxtools_init")()
	ASSERT(findtext(first, "FAILED") == 1)
	ASSERT(call_ext(auxtest_dll, "auxtools_init")() == first)

	var/reached_hook = TRUE
	try
		auxtest_hooks()
	catch
		reached_hook = FALSE
	ASSERT(!reached_hook)

/proc/concat_strings(a, b)
	return addtext(a, b)

/proc/text_lengths(t)
	return list(length(t), length_char(t), text2ascii_char(t, 2))

/proc/del_value(v)
	del v

var/datum/weak_kept_datum = new

var/datum/weak_test_datum
/proc/create_datum_for_weak()
	weak_test_datum = new

/mob/verb/auxtest_verb()
	return

/proc/auxtest_islist_samples()
	var/mob/M = new
	M.filters += filter(type = "blur")
	var/savefile/F = new
	var/list/L = list(1, 2)
	return list(
		null, 1, "string", M, /list, &L,
#if DM_VERSION >= 516
		alist("a" = 1),
#endif
		L, L.vars, args,
		M.verbs, M.contents, M.vars, M.overlays, M.group, M.vis_contents,
		M.filters, M.filters[1],
		F.dir, world.contents, world.vars, global.vars,
	)

#if DM_VERSION >= 516
/proc/auxtest_alist_sample()
	return alist("a" = 1, 7 = "seven", "n" = null)

/proc/auxtest_alist_keys(alist/A)
	var/list/keys = list()
	for (var/k in A)
		keys += list(k)
	return keys
#endif

/proc/auxtest_islist(value)
	return islist(value)

/proc/auxtest_length(value)
	return length(value)

/datum/auxtest_discard/New()
	CRASH()

/proc/auxtest_new_discard()
	new /datum/auxtest_discard

/proc/auxtest_hooked_global(x)
	CRASH()

/datum/auxtest_hooked/proc/hooked_method(x)
	CRASH()

/proc/auxtest_new_hooked()
	return new /datum/auxtest_hooked

/proc/auxtest_grow_proc_table()
	var/mob/M = new
	for (var/i in 1 to 4096)
		new /mob/verb/auxtest_verb(M, "auxtest clone [i]")

/proc/auxtest_skips_an_arg(first, second, third)
	CRASH()

/proc/auxtest_names_an_arg_args(first, second)
	CRASH()

/proc/auxtest_panics()
	CRASH()

/proc/auxtest_panic_is_caught(auxtest_dll)
	var/raised
	try
		auxtest_panics()
	catch(var/exception/e)
		raised = e.name
	ASSERT(findtext(raised, "panicked: auxtest asked for this panic"))
	ASSERT(call_ext(auxtest_dll, "auxtest_ffi_panics")() == "")

/proc/auxtest_hooks()
	CRASH()

/proc/auxtest_lists()
	CRASH()

/proc/auxtest_procs()
	CRASH()

/proc/auxtest_strings()
	CRASH()

/proc/auxtest_tables()
	CRASH()

/proc/auxtest_threads()
	CRASH()

/proc/auxtest_weak_values()
	CRASH()

/proc/auxtest_value_from()
	CRASH()

var/auxtest_big = 0
#define BIG1 auxtest_big += 1;
#define BIG4 BIG1 BIG1 BIG1 BIG1
#define BIG16 BIG4 BIG4 BIG4 BIG4
#define BIG64 BIG16 BIG16 BIG16 BIG16
#define BIG256 BIG64 BIG64 BIG64 BIG64
#define BIG1K BIG256 BIG256 BIG256 BIG256
#define BIG4K BIG1K BIG1K BIG1K BIG1K
#define BIG_MAX BIG4K BIG4K BIG4K BIG256 BIG256 BIG256 BIG16 BIG16 BIG16 BIG1 BIG1
#define BIG_TWINS(first, second) /proc/first() { BIG_MAX } /proc/second() { BIG_MAX }
BIG_TWINS(auxtest_big_a, auxtest_big_b)

/proc/auxtest_code_p()
	return "p"

/proc/auxtest_code_q()
	return "q"

/proc/auxtest_code_r()
	return "r"

/proc/auxtest_code_s()
	return "s"

/proc/auxtest_code_t()
	return "t"

/proc/auxtest_code_u()
	return "u"

/proc/auxtest_code_v()
	return "v"

/proc/auxtest_code_w()
	return "w"

/proc/auxtest_code_x()
	return "x"

/proc/auxtest_code_f()
	return "f"

/proc/auxtest_code_y()
	return "y"

/proc/auxtest_code_z()
	return "z"

/proc/auxtest_locals_user()
	var/unused = 1
	return unused

/proc/auxtest_code_before_shutdown()
	CRASH()

/proc/auxtest_shared_code()
	CRASH()

/proc/auxtest_entry_users()
	CRASH()

/proc/do_tests()
	var/auxtest_dll = auxtools_test_dll()
	var/init_result = call_ext(auxtest_dll, "auxtools_init")()
	world.log << "init_result = [init_result]"
	ASSERT(init_result == "SUCCESS")

	ASSERT(auxtest_hooks() == TRUE)
	auxtest_panic_is_caught(auxtest_dll)
	ASSERT(auxtest_lists() == TRUE)
	ASSERT(auxtest_procs() == TRUE)
	ASSERT(auxtest_strings() == TRUE)
	ASSERT(auxtest_tables() == TRUE)
	ASSERT(auxtest_threads() == TRUE)
	ASSERT(auxtest_value_from() == TRUE)
	ASSERT(auxtest_code_before_shutdown() == TRUE)
	ASSERT(auxtest_shared_code() == TRUE)
	ASSERT(auxtest_entry_users() == TRUE)

	var/datum/weak_test = new
	ASSERT(auxtest_weak_values(weak_test, weak_kept_datum) == TRUE)
	ASSERT(weak_test == null)

	if (auxtest_inc_counter() == 8)
		auxtest_shut_down(auxtest_dll)
		var/upgraded = call_ext(auxtest_dll, "auxtest_upgrade_kept_weak")()
		if (upgraded != "none")
			auxtest_out("FAILED: a weak reference upgraded after shutdown gave [upgraded ? upgraded : "an empty string"], expected none")
		call_ext(auxtest_dll, "auxtest_fail_next_init")()
		auxtest_failed_init(auxtest_dll)

		auxtest_out("SUCCESS: Finished")
		call_ext(auxtest_dll, "auxtools_shutdown")()
		shutdown()
	else
		auxtest_shut_down(auxtest_dll)
		world.Reboot()

/proc/auxtest_shut_down(auxtest_dll)
	var/shut_down = call_ext(auxtest_dll, "auxtools_shutdown")()
	if (shut_down != "SUCCESS")
		auxtest_out("FAILED: auxtools_shutdown gave [shut_down ? shut_down : "an empty string"], expected SUCCESS")
	var/after_shutdown = call_ext(auxtest_dll, "auxtest_after_shutdown")()
	if (after_shutdown != "ok")
		auxtest_out("FAILED: [after_shutdown ? after_shutdown : "the checks after shutdown returned nothing"]")
	var/init_again = call_ext(auxtest_dll, "auxtools_init")()
	if (init_again != "SUCCESS")
		auxtest_out("FAILED: auxtools_init after a shutdown gave [init_again ? init_again : "an empty string"], expected SUCCESS")
	var/restored = call_ext(auxtest_dll, "auxtest_code_after_shutdown")()
	if (restored != "SUCCESS")
		auxtest_out("FAILED: [restored ? restored : "the check of the code after shutdown gave an empty string"]")
	call_ext(auxtest_dll, "auxtools_shutdown")()

/world/New()
	do_tests()
	. = ..()

/world/Error(exception/e)
	auxtest_out("FAILED: world/Error([e])")
	. = ..()
	shutdown()
