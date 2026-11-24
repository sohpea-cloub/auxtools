// Guards: a_chain_through_fields_isnt_reused_after_a_change_as_a_value (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/proc/f(datum/a, b)
	return a.m.x++ + a.m.x
// Guards: a_const_chosen_by_a_constant_test_is_read_alone (compile_proc.rs)
/datum/t
	var/const/k = 1000
/proc/f()
	return (7 && /datum/t::k)
// Guards: a_dropped_if_forgets_the_cache (compile_proc.rs)
/datum/t
	var/list/l = list(1)
/proc/f(datum/t/o, a)
	o.l.Remove(a)
	if (0)
		a = 2
	return o.l.len
// Guards: a_false_else_if_leaves_its_jump (compile_proc.rs)
/proc/f(a, b)
	if (a) b = 1
	else if (0) b = 2
	return b
// Guards: a_chain_through_fields_isnt_reused_after_a_change_as_a_value (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/proc/f(datum/a, b)
	return (a.m.x += b) + a.m.x
// Guards: a_for_with_an_empty_first_part_is_read_past_it (compile_proc.rs)
/proc/f(a, b)
	for (; a < 5; a++)
		b++
	return b
// Guards: a_false_else_if_leaves_its_jump (compile_proc.rs)
/proc/f(a, b)
	if (0) b = 1
	else if (a) b = 2
	return b
// Guards: a_const_chosen_by_a_constant_test_is_read_alone (compile_proc.rs)
/datum/t
	var/const/k = 1000
/proc/f()
	return (/datum/t::k || "abc")
// Guards: a_global_const_named_through_global_folds (compile_proc.rs)
var/const/gc = 7
/datum/d
	var/const/gc = 9
/datum/d/proc/f(a)
	return global.gc + 1
// Guards: a_chain_through_fields_isnt_reused_after_a_change_as_a_value (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/proc/f(datum/a, b)
	return (a.m.x ||= b)
// Guards: a_for_with_an_empty_first_part_is_read_past_it (compile_proc.rs)
/proc/f(a, b)
	for (, a < 5, a++)
		b++
	return b
// Guards: a_logical_assignment_writes_its_field_as_it_read_it (compile_proc.rs)
/datum/d
	var/x
	var/y
/datum/d/proc/f(b)
	src.y = 1
	src.x &&= b
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	a
	return b
// Guards: a_const_chosen_by_a_constant_test_is_read_alone (compile_proc.rs)
/datum/t
	var/const/k = 1000
/proc/f()
	return (1 ? /datum/t::k : 2)
// Guards: a_chain_through_fields_isnt_reused_after_a_change_as_a_value (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/proc/f(datum/a, b)
	return a.m.x + (a.m.x = b)
// Guards: a_const_chosen_by_a_constant_test_is_read_alone (compile_proc.rs)
/datum/t
	var/const/k = 1000
/proc/f()
	return (0 || /datum/t::k + 1)
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = (5)
		return a.m.y
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	(a)
	return b
// Guards: a_global_const_named_through_global_folds (compile_proc.rs)
var/const/gc = 7
/datum/d
	var/const/gc = 9
/datum/d/proc/f(a)
	return ::gc + 1
// Guards: a_logical_assignment_writes_its_field_as_it_read_it (compile_proc.rs)
/datum/d
	var/x
	var/y
/datum/d/proc/f(b)
	y = 1
	src.x &&= b
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = ("s")
		return a.m.y
// Guards: a_name_alone_on_its_line_that_a_goto_names_is_a_label (compile_proc.rs)
/proc/f()
	var/i = 0
	again
	i++
	if (i < 5)
		goto again
	return i
// Guards: a_global_const_named_through_global_folds (compile_proc.rs)
var/const/gc = 7
/datum/d
	var/const/gc = 9
/datum/d/proc/f(a)
	return gc + 1
// Guards: a_logical_assignment_writes_its_field_as_it_read_it (compile_proc.rs)
/datum/d
	var/x
	var/y
/datum/d/proc/f(b)
	src.y = 1
	x ||= b
// Guards: a_parameter_default_stops_at_in (compile_proc.rs)
/proc/f(a, b = a in list(1), c = (a in list(1)))
	return b
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	a.x
	return b
// Guards: a_name_alone_on_its_line_that_a_goto_names_is_a_label (compile_proc.rs)
/proc/f(L)
	out
	for (var/x in L)
		if (x) goto out
// Guards: a_global_const_named_through_global_folds (compile_proc.rs)
var/const/gc = 7
/datum/d
	var/const/gc = 9
/datum/d/proc/f(a)
	return global.gc ? a : 2
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	usr
	return b
// Guards: a_pick_of_weight_0_and_a_case_of_minus_0 (compile_proc.rs)
/proc/f(a, b)
	return pick(1; a, 0; b, 2; "c")
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = (b)
		return a.m.y
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = (1 || b)
		return a.m.y
// Guards: a_global_const_named_through_global_folds (compile_proc.rs)
var/const/gc = 7
/datum/d
	var/const/gc = 9
/datum/d/proc/f(a)
	return global.gc
// Guards: a_pick_of_weight_0_and_a_case_of_minus_0 (compile_proc.rs)
// Known -516.1688: /proc/f  before 516.1689 DreamMaker's pick() ending in a zero weight jumps out of the proc; dmasm picks the last entry
/proc/f(a, b)
	return pick(1; a, 0; b)
// Guards: a_name_alone_on_its_line_that_a_goto_names_is_a_label (compile_proc.rs)
/proc/f(a)
	a
	return 1
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	null
	return b
// Guards: a_variable_times_itself_is_squared (compile_proc.rs)
var/g
/proc/f(a)
	return a * (a)
// Guards: a_global_const_named_through_global_folds (compile_proc.rs)
var/const/gc = 7
/datum/d
	var/const/gc = 9
/datum/d/proc/f(a)
	switch (a)
		if (global.gc)
			return 1
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = ("s" || b)
		return a.m.y
// Guards: a_pick_of_weight_0_and_a_case_of_minus_0 (compile_proc.rs)
/proc/f(a, b)
	return pick("x"; a, 3; b)
// Guards: a_variable_times_itself_is_squared (compile_proc.rs)
var/g
/proc/f(a)
	return g * global.g
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	isarea(a)
	return b
// Guards: alist_makes_an_alist (compile_proc.rs)
// Builds: 516.1648-
/proc/f(a)
	return alist(1 = a)
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = ("" && b)
		return a.m.y
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	a == b
	return b
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = (null && b)
		return a.m.y
// Guards: a_variable_times_itself_is_squared (compile_proc.rs)
var/g
/proc/f(a)
	return global.g * g
// Guards: a_line_that_does_nothing (compile_proc.rs)
/datum/var/x
/proc/f(datum/a, b)
	isnull(a)
	return b
// Guards: an_if_decided_when_compiled_forgets_the_cache_after_it (compile_proc.rs)
/datum/t
	var/a = 1
	var/b = 2
/proc/f(datum/t/p, n)
	if (1) n = p.a
	return p.b
// Guards: assert_gives_numbers_as_written (compile_proc.rs)
/proc/f(n)
	ASSERT(n > 1e3 + 0.50)
// Guards: a_variable_times_itself_is_squared (compile_proc.rs)
var/g
/proc/f(a)
	var/g = a
	return g * global.g
// Guards: an_implicit_new_for_a_list_of_a_type (compile_proc.rs)
/datum/d/var/datum/d/list/dl
/proc/f(datum/d/o)
	var/datum/d/list/v = new
	var/datum/d/list/w = new(3)
	o.dl = new
	return v
// Guards: a_logical_operator_folded_to_its_first_value_forgets_the_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a, b)
	if(a.m.x)
		var/c = (/datum || 1)
		return a.m.y
// Guards: assert_keeps_raw_strings_as_written (compile_proc.rs)
/proc/f(a)
	ASSERT(a == @{"brace"} || a == @'one')
// Guards: bit_not_of_huge_number (compile_proc.rs)
/proc/f()
	return ~(65536 ** 2)
// Guards: bit_ops_fold_like_dreammaker (compile_proc.rs)
/proc/f()
	. = ~(65536**3)
	. = ~(2**31+256)
	. = ~(-1)
	. = (2**33) & 16777215
	. = (-5) | 0
	return (2**31+256) >> 8
// Guards: builtin_const_not_folded (compile_proc.rs)
/atom/proc/f()
	return loc?.type || "x"
// Guards: call_arguments_are_a_procs (compile_proc.rs)
/proc/f(o, n)
	return call(o, "f")((n = 1), 2)
// Guards: call_ext_of_a_loaded_function (compile_proc.rs)
// Builds: 516.1648-
/proc/f()
	var/h = load_ext("lib.so", "func")
	return call_ext(h)(1, 2)
// Guards: chain_reloaded_after_call (compile_proc.rs)
/datum/var/datum/m
/datum/var/list/L
/datum/proc/g()
/proc/f(datum/a)
	a.m.g()
	for (var/x in a.m.L)
		return x
// Guards: cmptext_with_more_than_two (compile_proc.rs)
/proc/f()
	return cmptextEx("a", "A", "b")
// Guards: a_pick_of_weight_0_and_a_case_of_minus_0 (compile_proc.rs)
/proc/f(a)
	switch (a)
		if (-0)
			return 1
	return 2
// Guards: call_arguments_are_a_procs (compile_proc.rs)
/proc/f(o, n)
	return call(o, "f")((n = 1), x = 2)
// Guards: cmptext_with_more_than_two (compile_proc.rs)
/proc/f(a)
	return cmptext(a, "A", "b")
// Guards: chain_kept_after_call (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/proc/g()
/proc/f(datum/a)
	a.m.g()
	a.m.g()
	. = a.m.x
	. = a.m.x
// Guards: constant_lists_are_true_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (list(1, "k" = 2) ? a : b)
// Guards: const_alone_is_its_global_but_folds_in_expressions (compile_proc.rs)
var/const/RC = 20
/proc/f()
	world << RC
	world << RC + 1
	world << NORTH
// Guards: del_clears_its_target (compile_proc.rs)
/datum/q/proc/f()
	var/datum/x = new
	del(x)
	del src
// Guards: datum_loop_in_world_goes_over_atoms (compile_proc.rs)
/datum/foo
/proc/f()
	for (var/datum/foo/x in world)
		world.log << x
	for (var/datum/foo/y)
		world.log << y
// Guards: dropped_constant_forgets_cache (compile_proc.rs)
/datum/var/datum/m
/datum/var/x
/datum/var/y
/proc/f(datum/a)
	if(a.m.x)
		return (0 || a.m.y)
// Guards: empty_argument_is_pushed_null (compile_proc.rs)
/proc/g(x, y, z)
/proc/f(a, b)
	g(a, , b)
// Guards: constant_lists_are_true_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (list() ? a : b)
// Guards: folded_var_defaults_have_their_escapes (compile_proc.rs)
/datum/t
	var/v = "q\"q"
	var/const/k = "a\tb"
/proc/f()
	return ("a" + /datum/t::v) + /datum/t::k
// Guards: folding_and_or_forgets_the_cache (compile_proc.rs)
/datum/t
	var/a = 1
	var/list/l
/proc/f(datum/t/p, n)
	p.l = list()
	n = (1 || 2)
	return p.a
// Guards: findlasttext_defaults (compile_proc.rs)
/proc/f(a)
	return findlasttext(a, "/")
// Guards: constant_lists_are_true_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (new /list(2) ? a : b)
// Guards: goto_in_a_try_block_uses_try_jmp (compile_proc.rs)
/proc/f(a)
	try
		if (a) goto l1
		a = 2
		l1:
		a += 1
	catch
		if (a) goto l2
		l2:
	return a
// Guards: for_loop_separators_can_be_mixed (compile_proc.rs)
/proc/f(n)
	var/i
	for(i = 1; i <= n; i++)
		world.log << i
// Guards: constant_lists_are_true_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (newlist(/datum) ? a : b)
// Guards: hashes_of_text_fold (compile_proc.rs)
/proc/f()
	return md5("x")
// Guards: hex_escapes_are_characters (compile_proc.rs)
/proc/f()
	return "\xe9t\x41"
// Guards: for_loop_separators_can_be_mixed (compile_proc.rs)
/proc/f(n)
	var/i
	for(i = 1; i <= n, i++)
		world.log << i
// Guards: initial_of_a_static_field (compile_proc.rs)
/obj/item
	var/throwforce = 7
	var/static/sv = 3
	var/obj/item/other
/proc/a()
	return initial(/obj/item::throwforce)
/proc/b()
	return initial(/obj/item::sv)
/proc/c()
	var/obj/item/I = new
	return initial(I.other::throwforce)
/obj/item/proc/m()
	return initial(src::throwforce)
// Guards: constant_lists_are_true_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (list(k = new /datum) ? a : b)
// Guards: folding_and_or_forgets_the_cache (compile_proc.rs)
/datum/t
	var/a = 1
	var/list/l
/proc/f(datum/t/p, n)
	p.l = list()
	n = (1 ? 2 : 3)
	return p.a
// Guards: constant_lists_are_true_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (list(a) ? a : b)
// Guards: hashes_of_text_fold (compile_proc.rs)
/proc/f()
	return sha1("k" + "UPPER lower")
// Guards: initial_of_a_static_or_const_is_its_globals (compile_proc.rs)
/datum/t
	var/n = 1
	var/static/s = 2
	var/const/k = 3
	var/datum/t/o
/proc/f(datum/t/a)
	return initial(a.o.s) + initial(a.k)
// Guards: for_loop_separators_can_be_mixed (compile_proc.rs)
/proc/f(n)
	var/i
	for(i = 1, i <= n; i++)
		world.log << i
// Guards: hashes_of_text_fold (compile_proc.rs)
/proc/f()
	return md5("\xff")
// Guards: initial_of_a_static_or_const_is_its_globals (compile_proc.rs)
/datum/t
	var/n = 1
	var/static/s = 2
	var/const/k = 3
	var/datum/t/o
/proc/f(datum/t/a)
	return initial(a?.o?.n)
// Guards: input_from_builtin_sources (compile_proc.rs)
/proc/f(a, L)
	. = input(a) as num
	. = input(a) as mob in world
	. = input(a) in L
// Guards: hashes_of_text_fold (compile_proc.rs)
/proc/f()
	return md5(1)
// Guards: locate_in_a_list_in_another (compile_proc.rs)
/datum/t
/proc/f(a, b)
	return locate(/datum/t) in a in b
// Guards: missile_leaves_nothing (compile_proc.rs)
/proc/f()
	var/atom/a
	missile(/obj, a, a)
// Guards: logical_assignment_through_safe_access (compile_proc.rs)
// Known -516.1672: /proc/f  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/t
	var/v
	var/datum/t/n
/proc/f(datum/t/a, b)
	a?.n?.v ||= b
	return a
// Guards: new_without_arguments_is_true_in_a_test (compile_proc.rs)
/datum/t
/proc/f(a, b)
	return (new /datum/t ? a : b)
// Guards: new_without_arguments_is_true_in_a_test (compile_proc.rs)
/datum/t
/proc/f(a, b)
	return !(new /datum/t()) || (new /datum/t && a)
// Guards: logical_assignment_through_safe_access (compile_proc.rs)
// Known -516.1672: /proc/f  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/t
	var/v
	var/datum/t/n
/proc/f(datum/t/a, b)
	a?.v &&= a.n
// Guards: new_without_arguments_is_true_in_a_test (compile_proc.rs)
/datum/t
/proc/f(a, b)
	return (new /datum/t || a)
// Guards: or_assign_field_saves_cache_around_value (compile_proc.rs)
/datum/q
	var/x
	var/y
/proc/f(datum/q/o)
	o.y ||= o.x
// Guards: or_assign_goes_by_the_source_of_a_folded_value (compile_proc.rs)
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
/proc/f(datum/t/o)
	o.n &&= (0 && o.m)
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= o.st
// Guards: new_without_arguments_is_true_in_a_test (compile_proc.rs)
/datum/t
/proc/f(a, b)
	return (new /datum/t(a) ? a : b)
// Guards: parenthesised_assignment_is_a_list_key (compile_proc.rs)
/proc/f(a, b, c)
	return list(a, (b = 1), c)
// Guards: or_assign_goes_by_the_source_of_a_folded_value (compile_proc.rs)
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
/proc/f(datum/t/o)
	o.n &&= (0 && o.st)
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= o.k
// Guards: pointer_to_a_field (compile_proc.rs)
/datum/t
	var/v = 1
/proc/f()
	var/datum/t/T = new
	var/p = &T.v
	return *p
// Guards: parenthesised_assignment_is_a_list_key (compile_proc.rs)
/proc/f(a, b, c)
	return list(("x" = 1))
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= global.gx
// Guards: rarer_forms_of_builtins_on_their_own_line (compile_proc.rs)
/proc/f(a, b, c)
	statpanel(a, b)
	return a
// Guards: pointer_to_a_field (compile_proc.rs)
/datum/t
	var/v = 1
	proc/m()
		var/p = &src.v
		return *p
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= gx
// Guards: proc_argument_keys_worked_out (compile_proc.rs)
/proc/g(...)
/proc/f()
	var/a = 0
	return g(a + 1 = 2, 7)
// Guards: parenthesised_assignment_is_a_list_key (compile_proc.rs)
/proc/g(a, b)
	return b
/proc/f(a, b, c)
	return g(a, (b = 1))
// Guards: rarer_forms_of_builtins_on_their_own_line (compile_proc.rs)
/proc/f(a, b, c)
	statpanel(a, b, c)
	return a
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= o.m
// Guards: pointer_to_a_field (compile_proc.rs)
// Known -516.1667: /proc/f  before 516.1668 DreamMaker reads `*(&x)` as `*x`, a "bad pointer" error unless x holds a pointer (a BYOND bug, which dmasm warns of); dmasm reads x
/datum/t
	var/v = 1
/proc/f()
	var/datum/t/T = new
	var/p = &(T.v)
	return *(&T.v)
// Guards: proc_argument_keys_worked_out (compile_proc.rs)
/datum/t
	var/b
/proc/g(...)
/proc/f()
	var/datum/t/a = new
	return g(a?.b = 1)
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= L[1]
// Guards: rarer_forms_of_builtins_on_their_own_line (compile_proc.rs)
/proc/f(a, b, c)
	output(a, b)
	return a
// Guards: safe_access_assignment (compile_proc.rs)
/datum/q
	var/n
/proc/f(datum/q/a, x)
	a?.n = x
// Guards: pointer_to_a_field (compile_proc.rs)
/proc/f(a)
	var/p = &a
	return *(p)
// Guards: safe_access_to_a_const_is_not_folded (compile_proc.rs)
/datum/t
	var/const/k = 256
/proc/f(datum/t/a)
	if (a?.k) return 1
	return 2
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= /datum/t::st
// Guards: rarer_forms_of_builtins_on_their_own_line (compile_proc.rs)
/proc/f(a, b, c)
	animate(a, b, c)
	return a
// Guards: or_assign_saves_the_cache_for_some_values (compile_proc.rs)
var/gx = 1
/datum/t
	var/n = 1
	var/m = 2
	var/static/st = 3
	var/const/k = 4
/proc/f(datum/t/o, list/L)
	o.n ||= o::st
// Guards: savefile_input_statement (compile_proc.rs)
/proc/f(savefile/S, x)
	S["k"] >> x
// Guards: sized_list_var (compile_proc.rs)
/proc/f(n)
	var/list/L[n]
	return L
// Guards: a_block_local_const_is_folded_only_in_its_block (compile_proc.rs)
var/const/LIMIT = 10
/proc/f(x)
	if(x)
		var/const/LIMIT = 2
		x = LIMIT + 1
	return LIMIT + 1
// Guards: colon_reads_the_last_type_that_declares_its_var (compile_proc.rs)
/datum/q
/datum/p/a
	var/const/LIMIT = 5
/datum/p/b
	var/const/LIMIT = 9
/datum/p/b/c
/datum/q
	var/const/LIMIT = 20
	var/static/S = 4
/proc/f(x, datum/q/y, list/z)
	return list(x:LIMIT + 1, y:LIMIT + 1, z:LIMIT + 1, x:LIMIT, -y:LIMIT, x:S, x?:S)
// Guards: colon_chain_reads_the_type_of_each_var_it_goes_through (compile_proc.rs)
/datum/config
	var/const/LIMIT = 5
/datum/holder
	var/datum/config/config
/datum/other
	var/const/LIMIT = 9
/proc/f(x)
	return list(x:config:LIMIT + 1, x:config.LIMIT + 2)
// Guards: static_after_double_colon_chain (compile_proc.rs)
/datum/a
	var/datum/b/b
/datum/b
	var/const/C = 3
var/datum/a/A
/proc/f()
	return A::b::C
// Guards: switch_with_no_cases (compile_proc.rs)
/proc/f(a)
	switch(a)
	return 1
// Guards: text_squared_is_pow_square (compile_proc.rs)
/proc/f(x)
	. = "a" * "a"
	return "[x]" * "[x]"
// Guards: type_and_proc_macros_fold_in_a_test (compile_proc.rs)
/datum/t/proc/f(a, b)
	return (__TYPE__ ? a : b)
// Guards: type_macro_in_a_global_proc_is_null (compile_proc.rs)
/proc/f()
	return __TYPE__
// Guards: var_in_clause (compile_proc.rs)
/proc/f(a)
	var/x = a in list(1, 2)
	return x
// Guards: type_proc_reference_override (compile_proc.rs)
/atom/proc/p()
/obj/a/p()
/obj/a/b
/proc/f()
	return /obj/a/b::p()
// Guards: type_proc_reference_is_the_types_last_definition (compile_proc.rs)
/datum/t/proc/f()
/proc/g()
	return /datum/t::f()
// Guards: type_proc_reference_is_the_types_last_definition (compile_proc.rs)
/datum/t/proc/f()
/datum/t/f()
/datum/t/c
/proc/g()
	return /datum/t/c::f()
// Guards: type_proc_reference_is_the_types_last_definition (compile_proc.rs)
/datum/t/f()
/datum/t/proc/f()
/proc/g()
	return /datum/t::f()
// Guards: type_proc_reference_is_the_types_last_definition (compile_proc.rs)
/datum/t/proc/h()
/datum/t/f()
/datum/t/proc/f()
/proc/g()
	return /datum/t::f()
// Guards: type_proc_reference_is_the_types_last_definition (compile_proc.rs)
/icon/f()
/icon/proc/f()
/proc/g()
	return /icon::f()
// Guards: type_proc_reference_to_a_verb_is_its_override (compile_proc.rs)
/mob/verb/choose()
/mob/choose()
/mob/m
/proc/g()
	return /mob/m::choose()
// Guards: type_proc_reference_to_a_verb_is_its_override (compile_proc.rs)
/mob/choose()
/mob/verb/choose()
/proc/g()
	return /mob::choose()
// Guards: vars_after_a_switch_start_null (compile_proc.rs)
/proc/f(a)
	var/x
	switch(a)
		if(1)
			return
	var/y
	return list(x, y)
// Guards: verbs_are_called_by_display_name (compile_proc.rs)
/mob/verb/stop_pulling()
	set name = "Stop Pulling"
/proc/f(mob/m)
	m.stop_pulling()
// Guards: type_and_proc_macros_fold_in_a_test (compile_proc.rs)
/proc/f(a, b)
	return (__TYPE__ ? a : b)
// Guards: writing_a_static_var_through_safe_access (compile_proc.rs)
/datum/t
	var/static/s = 2
	var/datum/t/n
/proc/f(datum/t/a, b)
	a?.s = b
	a?.n.s += b
	b = a?.s++
// Guards: type_and_proc_macros_fold_in_a_test (compile_proc.rs)
// Builds: 515.1644-
/proc/f(a, b)
	return (__PROC__ ? a : b)
// Guards: writing_through_a_pointer (compile_proc.rs)
/proc/f(a)
	var/p = &a
	*p = 2
	return a
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	a?.f(1).x = b
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
// Known: /proc/g  DreamMaker calls f on what the value left in the cache; dmasm restores a
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	a?.f(1).x = a.n.x
// Guards: writing_through_a_pointer (compile_proc.rs)
/proc/f(a)
	var/p = &a
	*p += 2
	return a
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	return (a?.f(1).n.x = b)
// Guards: writing_through_a_pointer (compile_proc.rs)
/proc/f(a)
	var/p = &a
	return (*p = 3)
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	return a?.f(1).x++
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	a?.f(1).L[2] = b
// Guards: writing_through_a_pointer (compile_proc.rs)
/proc/f(a)
	var/p = &a
	(*p)++
	(*p) = 6
	return a
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	a?.f(1).L[2] += b
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
// Known -516.1672: /proc/g  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	a?.L[1].x ||= a.x
// Guards: writing_what_a_call_returns_through_safe_access (compile_proc.rs)
// Known -516.1672: /proc/g  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/d
	var/datum/d/n
	var/x
	var/list/L
/datum/d/proc/f(a)
	return n
/proc/g(datum/d/a, b)
	return (a?.f(1)[2] &&= b)
// Guards: an_assignment_operator_begins_with_dot_src (compile_proc.rs)
/datum/d
	var/n
/datum/d/proc/operator+=(b = 1)
	n = b
/datum/d/proc/operator-=(b = 1)
	n = b
/datum/d/proc/operator*=(b = 1)
	n = b
/datum/d/proc/operator/=(b = 1)
	n = b
/datum/d/proc/operator%=(b = 1)
	n = b
/datum/d/proc/operator%%=(b = 1)
	n = b
/datum/d/proc/operator&=(b = 1)
	n = b
/datum/d/proc/operator|=(b = 1)
	n = b
/datum/d/proc/operator^=(b = 1)
	n = b
/datum/d/proc/operator<<=(b = 1)
	n = b
/datum/d/proc/operator>>=(b = 1)
	n = b
/datum/d/proc/operator:=(b = 1)
	n = b
/datum/d/proc/operator++(b = 1)
	n = b
/datum/d/proc/operator--(b = 1)
	n = b
/datum/d/proc/operator+(b = 1)
	n = b
/datum/d/proc/operator-(b = 1)
	n = b
/datum/d/proc/operator*(b = 1)
	n = b
/datum/d/proc/operator/(b = 1)
	n = b
/datum/d/proc/operator%(b = 1)
	n = b
/datum/d/proc/operator%%(b = 1)
	n = b
/datum/d/proc/operator**(b = 1)
	n = b
/datum/d/proc/operator&(b = 1)
	n = b
/datum/d/proc/operator|(b = 1)
	n = b
/datum/d/proc/operator^(b = 1)
	n = b
/datum/d/proc/operator~(b = 1)
	n = b
/datum/d/proc/operator<<(b = 1)
	n = b
/datum/d/proc/operator>>(b = 1)
	n = b
/datum/d/proc/operator<(b = 1)
	n = b
/datum/d/proc/operator>(b = 1)
	n = b
/datum/d/proc/operator<=(b = 1)
	n = b
/datum/d/proc/operator>=(b = 1)
	n = b
/datum/d/proc/operator~=(b = 1)
	n = b
/datum/d/proc/operator~!(b = 1)
	n = b
/datum/d/proc/operator[](b = 1)
	n = b
/datum/d/proc/operator[]=(b = 1)
	n = b
/datum/d/proc/operator""(b = 1)
	n = b
// Guards: reading_a_pointer_it_makes_warns_before_516_1668 (compile_proc.rs)
// Known -516.1667: /datum/t/proc/m  before 516.1668 DreamMaker reads `*(&x)` as `*x`, a "bad pointer" error unless x holds a pointer (a BYOND bug, which dmasm warns of); dmasm reads x
// Known -516.1667: /proc/f  before 516.1668 DreamMaker reads `*(&x)` as `*x`, a "bad pointer" error unless x holds a pointer (a BYOND bug, which dmasm warns of); dmasm reads x
// Known -516.1667: /proc/w  before 516.1668 DreamMaker reads `*(&x)` as `*x`, a "bad pointer" error unless x holds a pointer (a BYOND bug, which dmasm warns of); dmasm reads x
/datum/t
	var/v = 1
	var/datum/t/w
var/g = 1
/datum/t/proc/m()
	return *(&src.v)
/proc/f(list/L)
	var/datum/t/T = new
	var/l = 1
	T.v = *&T.w.v
	return list(*(&l), *(&L[1]), *(&global.g))
/proc/w()
	var/datum/t/T = new
	*(&T.v) = 3
