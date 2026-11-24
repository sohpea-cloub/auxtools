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
// Guards: a_logical_assignment_through_a_safe_chain_writes_its_last_var_after_a_branching_value (compile_proc.rs)
// Known: /proc/f1  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f2  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
/datum/item
	var/datum/item/a
	var/datum/item/b
/proc/f1(datum/item/i, flag, x, y)
	i?.a.b ||= (flag ? x : y)
/proc/f2(datum/item/i, x, y)
	return (i?.a.b &&= (x || y))
// Guards: a_logical_assignment_to_a_safe_element_has_a_value (compile_proc.rs)
// Known -516.1672: /proc/f1  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f3  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/holder
	var/list/items
	var/datum/holder/next
/proc/f1(datum/holder/holder, i, fallback)
	return (holder?.items[i] ||= fallback)
/proc/f2(datum/holder/holder, i, fallback)
	return (holder?.next.items[i] &&= fallback)
/proc/f3(datum/holder/holder, i, fallback)
	return (holder?.next?.items[i] ||= fallback)
/proc/f4(datum/holder/holder, i, list/c, j)
	return (holder?.items[i] ||= c[j]++)
/proc/f5(list/items, i, fallback)
	return (items?[i] ||= fallback)
// Guards: a_logical_assignment_to_a_safe_element_keeps_its_key_across_the_value (compile_proc.rs)
// Known -516.1672: /proc/f1  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/item
	var/list/items
	var/list/counters
/proc/f1(datum/item/holder, i, j, list/counters)
	holder?.items[i] ||= counters[j]++
/proc/f2(datum/item/holder, i, j)
	holder?.items[i] &&= holder.counters[j]++
// Guards: a_logical_assignment_to_an_element_keeps_its_key_across_the_value (compile_proc.rs)
/datum/holder
	var/x
	var/list/items
/proc/f()
/proc/f1(list/items, i, j, list/counters)
	items[i] ||= counters[j]++
/proc/f2(list/items, i, j, list/counters)
	return (items[i] ||= counters[j]++)
/proc/f3(list/items, i, j, list/counters)
	items[i] &&= (counters[j] ||= 1)
/proc/f4(datum/holder/h, i, j, list/counters)
	return (h.items[i] ||= counters[j]++)
/proc/f5(list/items, i, j, list/counters)
	return (items[i] ||= counters[j])
/proc/f6(list/items, i)
	return (items[i] ||= f())
/proc/f7(datum/holder/h, list/c, j)
	return (h.x ||= c[j])
// Guards: a_logical_assignment_through_a_safe_chain_warns_where_dreammaker_writes_another_var (compile_proc.rs)
// Known: /proc/f1  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f2  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known -516.1672: /proc/f4  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known: /proc/f5  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
/datum/item
	var/datum/item/a
	var/datum/item/b
	var/datum/item/c
/proc/f1(datum/item/i, x)
	i?.a.b ||= x
/proc/f2(list/items, x)
	items?[1]:a:b &&= x
/proc/f3(datum/item/i, x)
	return (i?.a.b.c ||= x)
/proc/f4(datum/item/i, x)
	i?.a ||= x
/proc/f5(datum/item/i, datum/item/x)
	i?.a.b ||= x.c
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
// Guards: a_safe_chain_finds_a_var_after_an_index_as_untyped_colon_finds_it (compile_proc.rs)
/datum/a
	var/const/LIMIT = 5
	var/static/S = 1
	var/datum/a/link
/datum/b
	var/const/LIMIT = 9
	var/static/S = 2
	var/datum/b/link
/datum/holder
	var/list/datum/a/items
	var/list/plain
/proc/f1(datum/holder/holder)
	return holder?.items[1].LIMIT
/proc/f2(datum/holder/holder)
	return holder?.plain[1].LIMIT
/proc/f3(datum/holder/holder)
	return holder?.items[1]:LIMIT
/proc/f4(datum/holder/holder)
	return holder.items?[1].LIMIT
/proc/f5(datum/holder/holder)
	return holder?.plain[1].link.LIMIT
/proc/f6(datum/holder/holder)
	return holder?.items[1]?.LIMIT
/proc/f7(datum/holder/holder)
	return holder.items[1]?.LIMIT
/proc/f8(datum/holder/holder)
	return holder?.items[1].link?.LIMIT
/proc/f9(datum/holder/holder)
	return holder?.items[1].S
/proc/f10(datum/holder/holder)
	holder?.items[1].S = 3
/proc/f11(datum/holder/holder)
	holder?.items[1].S++
/proc/f12(datum/holder/holder)
	holder.items[1]?.S = 3
/proc/f13(datum/holder/holder)
	return holder?.items[1].LIMIT + 1
/proc/f14(datum/holder/holder)
	return holder?.items?[1].LIMIT
// Guards: a_var_after_a_call_is_found_by_the_procs_return_type (compile_proc.rs)
/datum/a
	var/const/LIMIT = 5
	var/static/S = 1
	var/datum/a/link
/datum/b
	var/const/LIMIT = 9
	var/static/S = 2
	var/datum/b/link
/datum/factory/proc/make()
	return new /datum/a
/datum/factory/proc/typed() as /datum/a
	return new /datum/a
/datum/factory/sub/typed()
	return new /datum/a
/proc/gmake()
	return new /datum/a
/proc/gtyped() as /datum/a
	return new /datum/a
/proc/f1(datum/factory/factory)
	return factory.make():LIMIT
/proc/f2(datum/factory/factory)
	return factory.make().LIMIT
/proc/f3(datum/factory/factory)
	return factory.typed():LIMIT
/proc/f4(datum/factory/factory)
	return factory.typed().LIMIT
/proc/f5(datum/factory/sub/factory)
	return factory.typed().LIMIT
/proc/f6(factory)
	return factory:typed().LIMIT
/proc/f7(datum/factory/factory)
	return factory.typed().link.LIMIT
/proc/f8(datum/factory/factory)
	return factory?.make().LIMIT
/proc/f9(datum/factory/factory)
	return factory?.typed().LIMIT
/proc/f10()
	return gmake().LIMIT
/proc/f11()
	return gtyped().LIMIT
/proc/f12(datum/factory/factory)
	return factory.make().S
/proc/f13(datum/factory/factory)
	return factory.make().link.LIMIT
/proc/f14(datum/factory/factory)
	return factory.typed()?.LIMIT
/proc/f15(datum/factory/factory)
	return factory.make()?.LIMIT
/proc/f16(datum/factory/factory)
	return factory.make():LIMIT + 1
// Guards: a_safe_write_calls_on_the_receiver_when_the_arguments_move_the_cache (compile_proc.rs)
// Known: /proc/f1  DreamMaker calls make() on what the arguments left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on holder
// Known: /proc/f2  DreamMaker calls make() on what the arguments left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on holder
// Known: /proc/f3  DreamMaker calls make() on what the arguments left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on holder
// Known: /proc/f4  DreamMaker calls make() on what the arguments left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on holder
/datum/holder
	var/flag
	var/datum/holder/child
/datum/holder/proc/make(x)
	return src
/proc/f1(datum/holder/holder, list/counters, i)
	holder?.make(counters[i]++).flag = 1
/proc/f2(datum/holder/holder, list/counters, i)
	holder?.child.make(counters[i]++).flag = 1
/proc/f3(datum/holder/holder, datum/holder/o)
	holder?.make(o.flag).flag = 1
/proc/f4(datum/holder/holder, list/counters, i)
	holder?.make(counters[i]--).flag ||= 1
// Guards: a_safe_write_calls_an_indexed_item_on_the_item (compile_proc.rs)
// Known -516.1672: /proc/f3  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/item
	var/flag
	var/datum/item/next
/datum/item/proc/make(x)
	return src
/datum/holder
	var/list/items
/proc/f1(datum/holder/holder, datum/item/other)
	holder?.items[1]:make().flag = 1
/proc/f2(datum/holder/holder, datum/item/other)
	holder?.items[1]:make(other.next).flag = 1
/proc/f3(datum/holder/holder, datum/item/other)
	holder?.items[1].make().flag ||= 1
// Guards: a_safe_write_after_a_call_finds_its_static_as_a_read_does (compile_proc.rs)
// Known -516.1672: /proc/f10  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/a
	var/static/S = 1
/datum/b
	var/static/S = 2
/datum/factory/proc/make()
	return new /datum/a
/datum/factory/proc/typed() as /datum/a
	return new /datum/a
/proc/gmake()
	return new /datum/a
/proc/gtyped() as /datum/a
	return new /datum/a
/proc/f1(datum/factory/factory)
	factory.make()?.S = 3
/proc/f2(datum/factory/factory)
	factory.typed()?.S = 3
/proc/f3()
	gmake()?.S = 3
/proc/f4()
	gtyped()?.S = 3
/proc/f5(datum/factory/factory)
	factory?.make().S = 3
/proc/f6(datum/factory/factory)
	factory?.typed().S = 3
/proc/f7(datum/factory/factory)
	factory.make()?.S += 3
/proc/f8(datum/factory/factory)
	factory.typed()?.S++
/proc/f9(datum/factory/factory)
	factory.make()?.S--
/proc/f10(datum/factory/factory)
	factory.make()?.S ||= 3
/proc/f11(datum/factory/factory)
	return (factory.typed()?.S = 4)
/proc/f12(datum/factory/factory)
	return factory.make()?.S++
// Guards: a_global_call_calls_the_global_proc_and_reads_by_its_return_type (compile_proc.rs)
/datum/a
	var/const/LIMIT = 5
	var/static/S = 1
/datum/b
	var/const/LIMIT = 9
	var/static/S = 2
/proc/make() as /datum/a
	return new /datum/a
/datum/thing/proc/make() as /datum/b
	return new /datum/b
/datum/thing/proc/f1()
	return ::make()
/datum/thing/proc/f2()
	return ::make().LIMIT
/datum/thing/proc/f3()
	return make()
/datum/thing/proc/f4()
	return ::make()?.LIMIT
/datum/thing/proc/f5()
	return ::make().S
/datum/thing/proc/f6()
	::make()?.S = 3
/datum/thing/proc/f7(a, b)
	return ::make(a, b)
/datum/thing/proc/f8()
	return ::make(arglist(list(1)))
/datum/thing/proc/f9()
	return ::make(a = 1)
/proc/f10()
	return ::make().LIMIT
// Guards: a_safe_chain_finds_each_calls_proc_and_value_by_the_type_before_it (compile_proc.rs)
/datum/a
	var/const/LIMIT = 5
	var/static/S = 1
/datum/b
	var/const/LIMIT = 9
	var/static/S = 2
/datum/decoy/proc/make() as /datum/b
	return new /datum/b
/datum/decoy/proc/named()
	return new /datum/b
/datum/decoy/verb/shown()
	return new /datum/b
/datum/builder/proc/make() as /datum/a
	return new /datum/b
/datum/builder/proc/named()
	set name = "build it"
	return new /datum/b
/datum/builder/proc/me() as /datum/builder
	return src
/datum/builder/proc/shown()
	return new /datum/b
/datum/factory/proc/builder() as /datum/builder
	return new /datum/builder
/proc/f1(datum/factory/factory)
	return factory?.builder().make().LIMIT
/proc/f2(datum/factory/factory)
	return factory?.builder().make().S
/proc/f3(datum/factory/factory)
	return factory?.builder().named()
/proc/f4(datum/factory/factory)
	return factory?.builder().make()?.LIMIT
/proc/f5(datum/factory/factory)
	return factory?.builder().make():LIMIT
/proc/f6(datum/factory/factory)
	factory?.builder().make().S = 4
/proc/f7(datum/factory/factory)
	return factory?.builder().me().make().LIMIT
/proc/f8(datum/factory/factory)
	return factory?.builder().me().named()
/proc/f9(datum/factory/factory)
	return factory?.builder().shown()
/proc/f10(datum/factory/factory)
	return factory?.builder():make().LIMIT
/proc/f11(datum/factory/factory)
	return factory?.builder().make().LIMIT + 1
// Guards: a_safe_write_finds_the_statics_it_goes_through_as_a_read_does (compile_proc.rs)
// Known -516.1672: /proc/f7  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f10  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f11  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/counter
	var/count = 0
	var/datum/counter/next
	var/static/datum/counter/INNER
/datum/a
	var/static/datum/counter/SHARED = new
	var/datum/counter/plain
	var/static/S = 1
/datum/b
	var/static/datum/counter/SHARED = new
	var/datum/counter/plain
	var/static/S = 2
/datum/holder
	var/datum/a/a
/proc/make()
	return new /datum/a
/proc/typed() as /datum/a
	return new /datum/b
/proc/f1()
	make()?.SHARED.count += 1
/proc/f2()
	typed()?.SHARED.count += 1
/proc/f3()
	make()?.SHARED.count = 1
/proc/f4()
	make()?.SHARED?.count = 1
/proc/f5()
	make()?.SHARED.next.count++
/proc/f6()
	return make()?.SHARED.count
/proc/f7()
	make()?.SHARED.count ||= 2
/proc/f8(datum/a/x)
	x?.SHARED.count += 1
/proc/f9()
	make()?.SHARED?.count += 1
/proc/f10()
	return (make()?.SHARED.count ||= 2)
/proc/f11()
	return (typed()?.S ||= 2)
/proc/f12(datum/holder/h)
	h?.a.SHARED.count += 1
/proc/f13(datum/a/x)
	x?.plain.INNER.count = 3
/proc/f14(datum/a/x)
	x?.SHARED.INNER.count = 3
/proc/f15(datum/a/x)
	x?.SHARED.count++
/proc/f16(datum/a/x)
	return x?.SHARED.count++
/proc/f17(datum/a/x)
	x?.SHARED.next?.count = 3
/proc/f18(datum/holder/h)
	h?.a?.SHARED.count = 1
/proc/f19(datum/a/x, i)
	x?.SHARED.next.count = 3
/proc/f20(x)
	x?:SHARED.count = 3
/proc/f21(datum/a/x)
	return (x?.SHARED.count += 1)
// Guards: a_safe_colon_call_on_an_untyped_value_finds_its_proc_as_a_colon_call_does (compile_proc.rs)
/mob/verb/show()
	set name = "Display"
/datum/thing/proc/show_proc()
	set name = "Show It"
/mob/verb/plain_verb()
/proc/f1(target)
	target?:show()
/proc/f2(target)
	target:show()
/proc/f3(target)
	target?:show_proc()
/proc/f5(target)
	target?:plain_verb()
/proc/f6(target)
	return target?:show_proc()
/proc/f7(target)
	target?:show().x = 1
/proc/f8(target)
	return target?:show()?.x
// Guards: a_colon_call_finds_its_proc_as_the_last_type_made_declares_it (compile_proc.rs)
/datum/t1
	var/const/LIMIT = 1
	var/x
/datum/t2
	var/LIMIT = 2
	var/x
/datum/zz
/datum/aa
/datum/zz/proc/make() as /datum/t1
	set name = "zz"
/datum/aa/proc/make() as /datum/t2
	set name = "aa"
/datum/cb
/datum/ca
/datum/cb/cc
/datum/ca/proc/tree_make()
	set name = "ca"
/datum/cb/cc/proc/tree_make()
	set name = "cb/cc"
/datum/holder/proc/act()
	set name = "holder act"
/mob/verb/act()
	set name = "mob act"
/datum/base
/datum/base/s2
/datum/base/s1
/datum/base/s1/proc/build()
	set name = "s1"
/datum/base/s2/proc/build()
	set name = "s2"
/datum/other/proc/build()
	set name = "other"
/datum/base/proc/own()
	set name = "base own"
/datum/other/proc/own()
	set name = "other own"
/proc/f1(x)
	return x:make()
/proc/f2(x)
	return x?:make()
/proc/f3(list/items)
	return items[1].make()
/proc/f4(x)
	return x:act()
/proc/f5(datum/base/base)
	return base:build()
/proc/f6(datum/base/base)
	return base?:build()
/proc/f7(datum/base/base)
	return base:own()
/proc/f8(datum/base/base)
	return base?:own()
/proc/f9(list/items)
	return items:make()
/proc/f10(x)
	return x:make().LIMIT
/proc/f11(x)
	return x?:make()?.LIMIT
/proc/f12(x)
	return x:tree_make()
/proc/f13(datum/base/base)
	base?:own().x = 1
/proc/f14(x)
	x:make()?:own().x = 1
// Guards: a_safe_colon_field_finds_its_var_as_on_an_untyped_value (compile_proc.rs)
/datum/base
	var/LIMIT = 1
/datum/later
	var/static/LIMIT = 2
/datum/later2
	var/const/COUNT = 3
/datum/base/sub
	var/const/COUNT = 4
/proc/f1(datum/base/sub/base)
	return base?:LIMIT
/proc/f2(datum/base/sub/base)
	base?:LIMIT = 5
/proc/f3(datum/base/sub/base)
	base?:LIMIT += 1
/proc/f4(datum/base/sub/base)
	return base:LIMIT
/proc/f5(datum/base/sub/base)
	return base?.LIMIT
/proc/f6(datum/base/sub/base)
	return base?:COUNT
// Guards: a_call_after_a_safe_index_keeps_the_indexed_item_as_its_receiver (compile_proc.rs)
/datum/item
	var/datum/item/owner
/datum/item/proc/refresh(a)
	return a
/proc/f1(list/items)
	return items?[1]:owner:refresh()
/proc/f2(list/items)
	return items?[1].owner.refresh()
/proc/f3(list/items, x)
	items?[1]:owner:refresh(x)
/proc/f4(list/items)
	return items?[1]:owner:owner:refresh()
/proc/f5(list/items)
	return items?[1]:owner:refresh(items?[2]:owner:refresh())
// Guards: a_call_after_a_safe_index_keeps_the_indexed_item_as_its_receiver (compile_proc.rs)
/datum/item/proc/refresh(a)
	return a
/datum/holder
	var/list/items
	var/datum/item/one
/proc/f1(list/items)
	return items?[1]:refresh()
/proc/f2(list/items)
	items?[1]:refresh()
/proc/f3(list/items, x)
	return items?[1]:refresh(x)
/proc/f4(list/items)
	return items?[1].refresh(items.len)
/proc/f5(list/items, list/other)
	return items?[1]:refresh(other?[1])
/proc/f6(list/items)
	return items?[1]:refresh()?:refresh()
/proc/f7(datum/holder/h)
	return h?.items[1].refresh(h.one)
// Guards: new_of_a_field_chain_reads_it_by_the_declared_type (compile_proc.rs)
/datum/item
/datum/item/special
/datum/config
	var/static/item_type = /datum/item/special
	var/plain_type = /datum/item
	var/const/CONST_TYPE = /datum/item/special
	var/datum/config/inner
/proc/f1(datum/config/config)
	return new config.item_type()
/proc/f2(datum/config/config)
	return new config.plain_type()
/proc/f3(datum/config/config)
	return new config.item_type(1)
/proc/f4(datum/config/config)
	return new config.inner.item_type()
/proc/f5(config)
	return new config:item_type()
/proc/f6(datum/config/config)
	return new config.CONST_TYPE()
/proc/f7(datum/config/config)
	var/datum/item/I = new config.item_type
	return I
/datum/config/proc/f8()
	return new src.item_type()
// Guards: a_safe_colon_call_finds_its_proc_and_value_as_a_colon_call_does (compile_proc.rs)
/datum/a
	var/const/LIMIT = 5
	var/static/S = 1
/datum/b
	var/const/LIMIT = 9
	var/static/S = 2
/datum/base
/datum/base/sub/proc/make() as /datum/a
	return new /datum/a
/datum/base/sub/proc/plain()
	return new /datum/a
/datum/base/sub/proc/named()
	set name = "build"
	return new /datum/a
/datum/base/sub/verb/vmake()
	return new /datum/a
/proc/f1(datum/base/base)
	return base?:make().LIMIT
/proc/f2(datum/base/base)
	return base:make().LIMIT
/proc/f3(datum/base/base)
	return base?:plain().LIMIT
/proc/f4(datum/base/base)
	return base?:named()
/proc/f5(datum/base/base)
	return base?:named().LIMIT
/proc/f6(datum/base/base)
	return base?:vmake().LIMIT
/proc/f7(datum/base/base)
	return base?:make().S
/proc/f8(datum/base/base)
	return base?:make()
/proc/f9(datum/base/base)
	base?:make().S = 3
/proc/f10(datum/base/base)
	return base?:make()?.LIMIT
/proc/f11(datum/base/base)
	base?:named().S = 3
/proc/f12(datum/base/base)
	base?:vmake().S = 3
/proc/f16(datum/base/base)
	base?:named()?.S = 3
/proc/f17(datum/base/base)
	return base?:make():LIMIT
// Guards: every_implicit_cast_in_a_value_takes_the_destinations_type (compile_proc.rs)
// Builds: 516.1648-
/datum/item
/datum/other
/datum/thing
	var/datum/item/owner
	var/datum/other/o
/proc/g(x, y)
	return x
/proc/f1(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = flag ? astype(a) : astype(b)
	return r
/proc/f2(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = astype(a) ? astype(b) : astype(a)
	return r
/proc/f3(flag, datum/other/a, datum/thing/b)
	var/datum/item/r
	r = flag ? astype(a) : astype(b)
	return r
/proc/f4(flag, datum/other/a, datum/thing/b)
	b.owner = flag ? astype(a) : astype(b)
/proc/f5(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = astype(a) + astype(b)
	return r
/proc/f6(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = g(astype(a), astype(b))
	return r
/proc/f7(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = astype(astype(a))
	return r
/proc/f8(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = new(astype(a))
	return r
/proc/f9(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = (astype(a) || astype(b)) + astype(b)
	return r
/proc/f10(flag, datum/other/a, datum/thing/b)
	var/datum/item/r
	r ||= flag ? astype(a) : astype(b)
	return r
/proc/f11(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = (b.o = astype(b)) + astype(a)
	return r
/proc/f12(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = (b.o ||= astype(b)) + astype(a)
	return r
/proc/f13(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = (b?.o = astype(b)) + astype(a)
	return r
/proc/f15(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = 1
	return astype(a)
// Guards: every_implicit_cast_in_a_value_takes_the_destinations_type (compile_proc.rs)
// Builds: 516.1673-
/datum/item
/datum/other
/datum/thing
	var/datum/item/owner
	var/datum/other/o
/proc/g(x, y)
	return x
/proc/f14(flag, datum/other/a, datum/thing/b)
	var/datum/item/r = (b?.o ||= astype(b)) + astype(a)
	return r
// Guards: every_implicit_cast_in_a_value_takes_the_destinations_type (compile_proc.rs)
/datum/item
/proc/f(flag)
	var/datum/item/r = flag ? locate() : locate()
	return r
// Guards: an_augmented_assignments_implicit_cast_takes_the_destinations_type (compile_proc.rs)
// Builds: 516.1648-
/datum/item
/datum/other
/datum/thing
	var/datum/item/o
	var/list/l
/proc/f1(datum/other/a, datum/thing/q)
	var/datum/item/x
	x += astype(a)
	return x
/proc/f2(datum/other/a, datum/thing/q)
	q.o -= astype(a)
/proc/f3(datum/other/a, datum/thing/q)
	var/datum/item/x
	x |= astype(a)
	return x
/proc/f4(datum/other/a, datum/thing/q)
	return (q.o += astype(a))
/proc/f5(datum/other/a, datum/thing/q)
	var/datum/other/r = (q.o += astype(a))
	return r
/proc/f6(datum/other/a, datum/thing/q)
	var/list/l = list()
	l += astype(a)
	return l
/proc/f7(datum/other/a, datum/thing/q)
	q.l[1] += astype(a)
/proc/f8(datum/other/a, datum/thing/q)
	q?.o += astype(a)
// Guards: an_augmented_assignments_implicit_cast_takes_the_destinations_type (compile_proc.rs)
/datum/item
/datum/other
/datum/thing
	var/datum/item/o
	var/list/l
/proc/f()
	var/datum/item/x = 1
	x += new
	return x
// Guards: a_named_arguments_place_key_types_its_value (compile_proc.rs)
// Builds: 516.1648-
/datum/item
/datum/other
/datum/thing
	var/datum/other/o
	var/list/l
	var/datum/item/i
/proc/g(x, y)
	return x
/proc/f1(datum/other/a, datum/thing/q)
	return g(q.o = astype(q))
/proc/f2(datum/other/a, datum/thing/q)
	var/datum/item/r = list(q.o = astype(q))
	return r
/proc/f3(datum/other/a, datum/thing/q)
	return g(q.l = astype(a))
/proc/f4(datum/other/a, datum/thing/q)
	return g(q.o[1] = astype(q))
/proc/f5(datum/other/a, datum/thing/q)
	return g(q?.o = astype(q))
/proc/f6(datum/other/a, datum/thing/q)
	return g(q.i = astype(a), q.o = astype(q))
/proc/f7(datum/other/a, datum/thing/q)
	return new /datum/other(q.o = astype(q))
/proc/f8(datum/other/a, datum/thing/q)
	return call(q, "x")(q.o = astype(q))
// Guards: a_named_arguments_place_key_types_its_value (compile_proc.rs)
/datum/item
/datum/other
/datum/thing
	var/datum/other/o
	var/list/l
	var/datum/item/i
/proc/g(x, y)
	return x
/proc/f(datum/thing/q)
	return list(q.o = new)
// Guards: an_elements_implicit_cast_takes_its_holders_type (compile_proc.rs)
// Builds: 516.1648-
/datum/item
/datum/other
	var/list/l
/datum/thing
	var/datum/other/o
	var/list/l
	var/list/datum/item/li
var/list/gl
/proc/f1(datum/other/a, datum/thing/q)
	q.o[1] = astype(q)
/proc/f2(datum/other/a, datum/thing/q)
	q.l[astype(q)] = 1
/proc/f3(datum/other/a, datum/thing/q)
	q.o.l[1] = astype(a)
/proc/f4(datum/other/a, datum/thing/q)
	q?.l[1] = astype(a)
/proc/f5(datum/other/a, datum/thing/q)
	gl[1] = astype(a)
/proc/f6(datum/other/a, datum/thing/q)
	args[1] = astype(a)
/proc/f7(datum/other/a, datum/thing/q)
	q.l[1] ||= astype(a)
/proc/f8(datum/other/a, datum/thing/q)
	return (q.o[1] = astype(q))
/proc/f9(datum/other/a, datum/thing/q)
	var/datum/item/r = (q.l[1] = astype(q))
	return r
/proc/f10(datum/other/a, datum/thing/q)
	var/list/l = list()
	l[1] = astype(a)
// Guards: an_elements_implicit_cast_takes_its_holders_type (compile_proc.rs)
/datum/item
/datum/other
	var/list/l
/datum/thing
	var/datum/other/o
	var/list/l
	var/list/datum/item/li
var/list/gl
/proc/f1(datum/thing/q)
	q.li[1] = new
/proc/f2(datum/thing/q)
	q.o[1] = new
/proc/f3(datum/thing/q)
	return (q.l[2] = new)
// Guards: a_for_loops_var_types_only_its_initial_value (compile_proc.rs)
// Builds: 516.1648-
/datum/item
/datum/other
/proc/f(datum/other/a)
	for(var/datum/item/r = 1; astype(a); r = null)
		return r
// Guards: a_safe_place_after_a_call_is_typed_as_a_read_finds_it (compile_proc.rs)
/datum/child
	var/datum/child/next
/datum/child/special
/datum/holder
	var/datum/holder/inner
	var/datum/child/child
	var/list/items
/datum/other
	var/datum/child/special/child
/datum/holder/proc/me() as /datum/holder
	return src
/datum/maker/proc/get_holder() as /datum/holder
	return new /datum/holder
/datum/maker/proc/untyped_holder()
	return new /datum/holder
/proc/get_holder() as /datum/holder
	return new /datum/holder
/proc/f1()
	get_holder()?.child = new
/proc/f2(datum/maker/maker)
	maker.get_holder()?.child = new
/proc/f3(datum/maker/maker)
	maker.untyped_holder()?.child = new
/proc/f4(datum/maker/maker)
	maker.get_holder()?:child = new
/proc/f5(datum/maker/maker)
	maker?.get_holder()?.child = new
/proc/f6()
	get_holder()?.child.next = new
/proc/f7()
	get_holder().inner?.child = new
/proc/f8()
	get_holder()?.inner.child = new
/proc/f9(datum/holder/h)
	h.me()?.child = new
/proc/f10()
	get_holder()?.items = new
/proc/f11()
	get_holder()?.child = new()
/proc/f12()
	return (get_holder()?.child = new)
/proc/f13(datum/holder/h)
	h.inner.child = new
// Guards: a_safe_place_after_a_call_is_typed_as_a_read_finds_it (compile_proc.rs)
// Builds: 516.1648-
/datum/child
/datum/child/special
/datum/holder
	var/datum/child/child
/datum/other
	var/datum/child/special/child
/datum/maker/proc/untyped_holder()
	return new /datum/holder
/proc/get_holder() as /datum/holder
	return new /datum/holder
/proc/f1(x)
	get_holder()?.child = astype(x)
/proc/f2()
	return astype(get_holder().child)
/proc/f3(datum/maker/maker)
	return astype(maker.untyped_holder().child)
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
// Guards: every assignment through `?.`: one to three, ending in a var, an element or a call, `=`, `+=`, `||=`, `&&=` and `++`, as a statement and a value (no test of its own)
// Known -516.1672: /proc/f1_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f1_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f2_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known: /proc/f3_or_s  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_or_sm  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_or_v  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_or_vm  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_and_s  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_and_sm  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_and_v  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known: /proc/f3_and_vm  DreamMaker writes `a?.b.c ||= v` through the object its test read, a.b.b.c (a BYOND bug, which dmasm warns of); dmasm writes a.b.c
// Known -516.1672: /proc/f4_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f4_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/f5_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e1_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e2_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e3_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e4_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/e5_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known: /proc/c1_set_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c1_set_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c1_add_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c1_add_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known -516.1672: /proc/c1_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c1_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known: /proc/c2_set_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c2_set_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c2_add_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c2_add_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known -516.1672: /proc/c2_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c2_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c3_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known: /proc/c4_set_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c4_set_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c4_add_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c4_add_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known -516.1672: /proc/c4_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c4_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known: /proc/c5_set_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c5_set_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c5_add_sm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known: /proc/c5_add_vm  DreamMaker calls m() on what the value left in the cache (a BYOND bug, which dmasm warns of); dmasm calls it on h
// Known -516.1672: /proc/c5_or_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_or_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_or_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_or_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_and_s  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_and_sm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_and_v  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
// Known -516.1672: /proc/c5_and_vm  before 516.1673 DreamMaker leaves out the null tests of `a?.b ||= v` and `&&=`; dmasm keeps them
/datum/h
	var/v
	var/datum/h/c
	var/list/l
/datum/h/proc/m()
	return src
/proc/f1_set_s(datum/h/h, list/l, i, x)
	h?.v = x
/proc/f1_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.v = x.v
/proc/f1_set_v(datum/h/h, list/l, i, x)
	return (h?.v = x)
/proc/f1_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.v = x.v)
/proc/f1_add_s(datum/h/h, list/l, i, x)
	h?.v += x
/proc/f1_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.v += x.v
/proc/f1_add_v(datum/h/h, list/l, i, x)
	return (h?.v += x)
/proc/f1_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.v += x.v)
/proc/f1_or_s(datum/h/h, list/l, i, x)
	h?.v ||= x
/proc/f1_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.v ||= x.v
/proc/f1_or_v(datum/h/h, list/l, i, x)
	return (h?.v ||= x)
/proc/f1_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.v ||= x.v)
/proc/f1_and_s(datum/h/h, list/l, i, x)
	h?.v &&= x
/proc/f1_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.v &&= x.v
/proc/f1_and_v(datum/h/h, list/l, i, x)
	return (h?.v &&= x)
/proc/f1_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.v &&= x.v)
/proc/f2_set_s(datum/h/h, list/l, i, x)
	h?.c?.v = x
/proc/f2_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.v = x.v
/proc/f2_set_v(datum/h/h, list/l, i, x)
	return (h?.c?.v = x)
/proc/f2_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.v = x.v)
/proc/f2_add_s(datum/h/h, list/l, i, x)
	h?.c?.v += x
/proc/f2_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.v += x.v
/proc/f2_add_v(datum/h/h, list/l, i, x)
	return (h?.c?.v += x)
/proc/f2_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.v += x.v)
/proc/f2_or_s(datum/h/h, list/l, i, x)
	h?.c?.v ||= x
/proc/f2_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.v ||= x.v
/proc/f2_or_v(datum/h/h, list/l, i, x)
	return (h?.c?.v ||= x)
/proc/f2_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.v ||= x.v)
/proc/f2_and_s(datum/h/h, list/l, i, x)
	h?.c?.v &&= x
/proc/f2_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.v &&= x.v
/proc/f2_and_v(datum/h/h, list/l, i, x)
	return (h?.c?.v &&= x)
/proc/f2_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.v &&= x.v)
/proc/f3_set_s(datum/h/h, list/l, i, x)
	h?.c.v = x
/proc/f3_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.v = x.v
/proc/f3_set_v(datum/h/h, list/l, i, x)
	return (h?.c.v = x)
/proc/f3_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.v = x.v)
/proc/f3_add_s(datum/h/h, list/l, i, x)
	h?.c.v += x
/proc/f3_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.v += x.v
/proc/f3_add_v(datum/h/h, list/l, i, x)
	return (h?.c.v += x)
/proc/f3_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.v += x.v)
/proc/f3_or_s(datum/h/h, list/l, i, x)
	h?.c.v ||= x
/proc/f3_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.v ||= x.v
/proc/f3_or_v(datum/h/h, list/l, i, x)
	return (h?.c.v ||= x)
/proc/f3_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.v ||= x.v)
/proc/f3_and_s(datum/h/h, list/l, i, x)
	h?.c.v &&= x
/proc/f3_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.v &&= x.v
/proc/f3_and_v(datum/h/h, list/l, i, x)
	return (h?.c.v &&= x)
/proc/f3_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.v &&= x.v)
/proc/f4_set_s(datum/h/h, list/l, i, x)
	h.c?.v = x
/proc/f4_set_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.v = x.v
/proc/f4_set_v(datum/h/h, list/l, i, x)
	return (h.c?.v = x)
/proc/f4_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.v = x.v)
/proc/f4_add_s(datum/h/h, list/l, i, x)
	h.c?.v += x
/proc/f4_add_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.v += x.v
/proc/f4_add_v(datum/h/h, list/l, i, x)
	return (h.c?.v += x)
/proc/f4_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.v += x.v)
/proc/f4_or_s(datum/h/h, list/l, i, x)
	h.c?.v ||= x
/proc/f4_or_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.v ||= x.v
/proc/f4_or_v(datum/h/h, list/l, i, x)
	return (h.c?.v ||= x)
/proc/f4_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.v ||= x.v)
/proc/f4_and_s(datum/h/h, list/l, i, x)
	h.c?.v &&= x
/proc/f4_and_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.v &&= x.v
/proc/f4_and_v(datum/h/h, list/l, i, x)
	return (h.c?.v &&= x)
/proc/f4_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.v &&= x.v)
/proc/f5_set_s(datum/h/h, list/l, i, x)
	h?.c?.c?.v = x
/proc/f5_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.c?.v = x.v
/proc/f5_set_v(datum/h/h, list/l, i, x)
	return (h?.c?.c?.v = x)
/proc/f5_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.c?.v = x.v)
/proc/f5_add_s(datum/h/h, list/l, i, x)
	h?.c?.c?.v += x
/proc/f5_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.c?.v += x.v
/proc/f5_add_v(datum/h/h, list/l, i, x)
	return (h?.c?.c?.v += x)
/proc/f5_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.c?.v += x.v)
/proc/f5_or_s(datum/h/h, list/l, i, x)
	h?.c?.c?.v ||= x
/proc/f5_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.c?.v ||= x.v
/proc/f5_or_v(datum/h/h, list/l, i, x)
	return (h?.c?.c?.v ||= x)
/proc/f5_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.c?.v ||= x.v)
/proc/f5_and_s(datum/h/h, list/l, i, x)
	h?.c?.c?.v &&= x
/proc/f5_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.c?.v &&= x.v
/proc/f5_and_v(datum/h/h, list/l, i, x)
	return (h?.c?.c?.v &&= x)
/proc/f5_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.c?.v &&= x.v)
/proc/e1_set_s(datum/h/h, list/l, i, x)
	h?.l[i] = x
/proc/e1_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.l[i] = x.v
/proc/e1_set_v(datum/h/h, list/l, i, x)
	return (h?.l[i] = x)
/proc/e1_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.l[i] = x.v)
/proc/e1_add_s(datum/h/h, list/l, i, x)
	h?.l[i] += x
/proc/e1_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.l[i] += x.v
/proc/e1_add_v(datum/h/h, list/l, i, x)
	return (h?.l[i] += x)
/proc/e1_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.l[i] += x.v)
/proc/e1_or_s(datum/h/h, list/l, i, x)
	h?.l[i] ||= x
/proc/e1_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.l[i] ||= x.v
/proc/e1_or_v(datum/h/h, list/l, i, x)
	return (h?.l[i] ||= x)
/proc/e1_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.l[i] ||= x.v)
/proc/e1_and_s(datum/h/h, list/l, i, x)
	h?.l[i] &&= x
/proc/e1_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.l[i] &&= x.v
/proc/e1_and_v(datum/h/h, list/l, i, x)
	return (h?.l[i] &&= x)
/proc/e1_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.l[i] &&= x.v)
/proc/e2_set_s(datum/h/h, list/l, i, x)
	h?.c?.l[i] = x
/proc/e2_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.l[i] = x.v
/proc/e2_set_v(datum/h/h, list/l, i, x)
	return (h?.c?.l[i] = x)
/proc/e2_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.l[i] = x.v)
/proc/e2_add_s(datum/h/h, list/l, i, x)
	h?.c?.l[i] += x
/proc/e2_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.l[i] += x.v
/proc/e2_add_v(datum/h/h, list/l, i, x)
	return (h?.c?.l[i] += x)
/proc/e2_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.l[i] += x.v)
/proc/e2_or_s(datum/h/h, list/l, i, x)
	h?.c?.l[i] ||= x
/proc/e2_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.l[i] ||= x.v
/proc/e2_or_v(datum/h/h, list/l, i, x)
	return (h?.c?.l[i] ||= x)
/proc/e2_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.l[i] ||= x.v)
/proc/e2_and_s(datum/h/h, list/l, i, x)
	h?.c?.l[i] &&= x
/proc/e2_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.l[i] &&= x.v
/proc/e2_and_v(datum/h/h, list/l, i, x)
	return (h?.c?.l[i] &&= x)
/proc/e2_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.l[i] &&= x.v)
/proc/e3_set_s(datum/h/h, list/l, i, x)
	h?.c.l[i] = x
/proc/e3_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.l[i] = x.v
/proc/e3_set_v(datum/h/h, list/l, i, x)
	return (h?.c.l[i] = x)
/proc/e3_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.l[i] = x.v)
/proc/e3_add_s(datum/h/h, list/l, i, x)
	h?.c.l[i] += x
/proc/e3_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.l[i] += x.v
/proc/e3_add_v(datum/h/h, list/l, i, x)
	return (h?.c.l[i] += x)
/proc/e3_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.l[i] += x.v)
/proc/e3_or_s(datum/h/h, list/l, i, x)
	h?.c.l[i] ||= x
/proc/e3_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.l[i] ||= x.v
/proc/e3_or_v(datum/h/h, list/l, i, x)
	return (h?.c.l[i] ||= x)
/proc/e3_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.l[i] ||= x.v)
/proc/e3_and_s(datum/h/h, list/l, i, x)
	h?.c.l[i] &&= x
/proc/e3_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.l[i] &&= x.v
/proc/e3_and_v(datum/h/h, list/l, i, x)
	return (h?.c.l[i] &&= x)
/proc/e3_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.l[i] &&= x.v)
/proc/e4_set_s(datum/h/h, list/l, i, x)
	l?[i] = x
/proc/e4_set_sm(datum/h/h, list/l, i, datum/h/x)
	l?[i] = x.v
/proc/e4_set_v(datum/h/h, list/l, i, x)
	return (l?[i] = x)
/proc/e4_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (l?[i] = x.v)
/proc/e4_add_s(datum/h/h, list/l, i, x)
	l?[i] += x
/proc/e4_add_sm(datum/h/h, list/l, i, datum/h/x)
	l?[i] += x.v
/proc/e4_add_v(datum/h/h, list/l, i, x)
	return (l?[i] += x)
/proc/e4_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (l?[i] += x.v)
/proc/e4_or_s(datum/h/h, list/l, i, x)
	l?[i] ||= x
/proc/e4_or_sm(datum/h/h, list/l, i, datum/h/x)
	l?[i] ||= x.v
/proc/e4_or_v(datum/h/h, list/l, i, x)
	return (l?[i] ||= x)
/proc/e4_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (l?[i] ||= x.v)
/proc/e4_and_s(datum/h/h, list/l, i, x)
	l?[i] &&= x
/proc/e4_and_sm(datum/h/h, list/l, i, datum/h/x)
	l?[i] &&= x.v
/proc/e4_and_v(datum/h/h, list/l, i, x)
	return (l?[i] &&= x)
/proc/e4_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (l?[i] &&= x.v)
/proc/e5_set_s(datum/h/h, list/l, i, x)
	h.c?.l[i] = x
/proc/e5_set_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.l[i] = x.v
/proc/e5_set_v(datum/h/h, list/l, i, x)
	return (h.c?.l[i] = x)
/proc/e5_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.l[i] = x.v)
/proc/e5_add_s(datum/h/h, list/l, i, x)
	h.c?.l[i] += x
/proc/e5_add_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.l[i] += x.v
/proc/e5_add_v(datum/h/h, list/l, i, x)
	return (h.c?.l[i] += x)
/proc/e5_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.l[i] += x.v)
/proc/e5_or_s(datum/h/h, list/l, i, x)
	h.c?.l[i] ||= x
/proc/e5_or_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.l[i] ||= x.v
/proc/e5_or_v(datum/h/h, list/l, i, x)
	return (h.c?.l[i] ||= x)
/proc/e5_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.l[i] ||= x.v)
/proc/e5_and_s(datum/h/h, list/l, i, x)
	h.c?.l[i] &&= x
/proc/e5_and_sm(datum/h/h, list/l, i, datum/h/x)
	h.c?.l[i] &&= x.v
/proc/e5_and_v(datum/h/h, list/l, i, x)
	return (h.c?.l[i] &&= x)
/proc/e5_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h.c?.l[i] &&= x.v)
/proc/c1_set_s(datum/h/h, list/l, i, x)
	h?.m().v = x
/proc/c1_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().v = x.v
/proc/c1_set_v(datum/h/h, list/l, i, x)
	return (h?.m().v = x)
/proc/c1_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().v = x.v)
/proc/c1_add_s(datum/h/h, list/l, i, x)
	h?.m().v += x
/proc/c1_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().v += x.v
/proc/c1_add_v(datum/h/h, list/l, i, x)
	return (h?.m().v += x)
/proc/c1_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().v += x.v)
/proc/c1_or_s(datum/h/h, list/l, i, x)
	h?.m().v ||= x
/proc/c1_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().v ||= x.v
/proc/c1_or_v(datum/h/h, list/l, i, x)
	return (h?.m().v ||= x)
/proc/c1_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().v ||= x.v)
/proc/c1_and_s(datum/h/h, list/l, i, x)
	h?.m().v &&= x
/proc/c1_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().v &&= x.v
/proc/c1_and_v(datum/h/h, list/l, i, x)
	return (h?.m().v &&= x)
/proc/c1_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().v &&= x.v)
/proc/c2_set_s(datum/h/h, list/l, i, x)
	h?.c?.m().v = x
/proc/c2_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.m().v = x.v
/proc/c2_set_v(datum/h/h, list/l, i, x)
	return (h?.c?.m().v = x)
/proc/c2_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.m().v = x.v)
/proc/c2_add_s(datum/h/h, list/l, i, x)
	h?.c?.m().v += x
/proc/c2_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.m().v += x.v
/proc/c2_add_v(datum/h/h, list/l, i, x)
	return (h?.c?.m().v += x)
/proc/c2_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.m().v += x.v)
/proc/c2_or_s(datum/h/h, list/l, i, x)
	h?.c?.m().v ||= x
/proc/c2_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.m().v ||= x.v
/proc/c2_or_v(datum/h/h, list/l, i, x)
	return (h?.c?.m().v ||= x)
/proc/c2_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.m().v ||= x.v)
/proc/c2_and_s(datum/h/h, list/l, i, x)
	h?.c?.m().v &&= x
/proc/c2_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c?.m().v &&= x.v
/proc/c2_and_v(datum/h/h, list/l, i, x)
	return (h?.c?.m().v &&= x)
/proc/c2_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c?.m().v &&= x.v)
/proc/c3_set_s(datum/h/h, list/l, i, x)
	h?.m()?.v = x
/proc/c3_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m()?.v = x.v
/proc/c3_set_v(datum/h/h, list/l, i, x)
	return (h?.m()?.v = x)
/proc/c3_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m()?.v = x.v)
/proc/c3_add_s(datum/h/h, list/l, i, x)
	h?.m()?.v += x
/proc/c3_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m()?.v += x.v
/proc/c3_add_v(datum/h/h, list/l, i, x)
	return (h?.m()?.v += x)
/proc/c3_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m()?.v += x.v)
/proc/c3_or_s(datum/h/h, list/l, i, x)
	h?.m()?.v ||= x
/proc/c3_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m()?.v ||= x.v
/proc/c3_or_v(datum/h/h, list/l, i, x)
	return (h?.m()?.v ||= x)
/proc/c3_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m()?.v ||= x.v)
/proc/c3_and_s(datum/h/h, list/l, i, x)
	h?.m()?.v &&= x
/proc/c3_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m()?.v &&= x.v
/proc/c3_and_v(datum/h/h, list/l, i, x)
	return (h?.m()?.v &&= x)
/proc/c3_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m()?.v &&= x.v)
/proc/c4_set_s(datum/h/h, list/l, i, x)
	h?.m().l[i] = x
/proc/c4_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().l[i] = x.v
/proc/c4_set_v(datum/h/h, list/l, i, x)
	return (h?.m().l[i] = x)
/proc/c4_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().l[i] = x.v)
/proc/c4_add_s(datum/h/h, list/l, i, x)
	h?.m().l[i] += x
/proc/c4_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().l[i] += x.v
/proc/c4_add_v(datum/h/h, list/l, i, x)
	return (h?.m().l[i] += x)
/proc/c4_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().l[i] += x.v)
/proc/c4_or_s(datum/h/h, list/l, i, x)
	h?.m().l[i] ||= x
/proc/c4_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().l[i] ||= x.v
/proc/c4_or_v(datum/h/h, list/l, i, x)
	return (h?.m().l[i] ||= x)
/proc/c4_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().l[i] ||= x.v)
/proc/c4_and_s(datum/h/h, list/l, i, x)
	h?.m().l[i] &&= x
/proc/c4_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.m().l[i] &&= x.v
/proc/c4_and_v(datum/h/h, list/l, i, x)
	return (h?.m().l[i] &&= x)
/proc/c4_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.m().l[i] &&= x.v)
/proc/c5_set_s(datum/h/h, list/l, i, x)
	h?.c.m().v = x
/proc/c5_set_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.m().v = x.v
/proc/c5_set_v(datum/h/h, list/l, i, x)
	return (h?.c.m().v = x)
/proc/c5_set_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.m().v = x.v)
/proc/c5_add_s(datum/h/h, list/l, i, x)
	h?.c.m().v += x
/proc/c5_add_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.m().v += x.v
/proc/c5_add_v(datum/h/h, list/l, i, x)
	return (h?.c.m().v += x)
/proc/c5_add_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.m().v += x.v)
/proc/c5_or_s(datum/h/h, list/l, i, x)
	h?.c.m().v ||= x
/proc/c5_or_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.m().v ||= x.v
/proc/c5_or_v(datum/h/h, list/l, i, x)
	return (h?.c.m().v ||= x)
/proc/c5_or_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.m().v ||= x.v)
/proc/c5_and_s(datum/h/h, list/l, i, x)
	h?.c.m().v &&= x
/proc/c5_and_sm(datum/h/h, list/l, i, datum/h/x)
	h?.c.m().v &&= x.v
/proc/c5_and_v(datum/h/h, list/l, i, x)
	return (h?.c.m().v &&= x)
/proc/c5_and_vm(datum/h/h, list/l, i, datum/h/x)
	return (h?.c.m().v &&= x.v)
/proc/f1_inc_s(datum/h/h, list/l, i, x)
	h?.v++
/proc/f1_inc_v(datum/h/h, list/l, i, x)
	return (h?.v++)
/proc/f2_inc_s(datum/h/h, list/l, i, x)
	h?.c?.v++
/proc/f2_inc_v(datum/h/h, list/l, i, x)
	return (h?.c?.v++)
/proc/f3_inc_s(datum/h/h, list/l, i, x)
	h?.c.v++
/proc/f3_inc_v(datum/h/h, list/l, i, x)
	return (h?.c.v++)
/proc/f4_inc_s(datum/h/h, list/l, i, x)
	h.c?.v++
/proc/f4_inc_v(datum/h/h, list/l, i, x)
	return (h.c?.v++)
/proc/f5_inc_s(datum/h/h, list/l, i, x)
	h?.c?.c?.v++
/proc/f5_inc_v(datum/h/h, list/l, i, x)
	return (h?.c?.c?.v++)
/proc/e1_inc_s(datum/h/h, list/l, i, x)
	h?.l[i]++
/proc/e1_inc_v(datum/h/h, list/l, i, x)
	return (h?.l[i]++)
/proc/e2_inc_s(datum/h/h, list/l, i, x)
	h?.c?.l[i]++
/proc/e2_inc_v(datum/h/h, list/l, i, x)
	return (h?.c?.l[i]++)
/proc/e3_inc_s(datum/h/h, list/l, i, x)
	h?.c.l[i]++
/proc/e3_inc_v(datum/h/h, list/l, i, x)
	return (h?.c.l[i]++)
/proc/e4_inc_s(datum/h/h, list/l, i, x)
	l?[i]++
/proc/e4_inc_v(datum/h/h, list/l, i, x)
	return (l?[i]++)
/proc/e5_inc_s(datum/h/h, list/l, i, x)
	h.c?.l[i]++
/proc/e5_inc_v(datum/h/h, list/l, i, x)
	return (h.c?.l[i]++)
/proc/c1_inc_s(datum/h/h, list/l, i, x)
	h?.m().v++
/proc/c1_inc_v(datum/h/h, list/l, i, x)
	return (h?.m().v++)
/proc/c2_inc_s(datum/h/h, list/l, i, x)
	h?.c?.m().v++
/proc/c2_inc_v(datum/h/h, list/l, i, x)
	return (h?.c?.m().v++)
/proc/c3_inc_s(datum/h/h, list/l, i, x)
	h?.m()?.v++
/proc/c3_inc_v(datum/h/h, list/l, i, x)
	return (h?.m()?.v++)
/proc/c4_inc_s(datum/h/h, list/l, i, x)
	h?.m().l[i]++
/proc/c4_inc_v(datum/h/h, list/l, i, x)
	return (h?.m().l[i]++)
/proc/c5_inc_s(datum/h/h, list/l, i, x)
	h?.c.m().v++
/proc/c5_inc_v(datum/h/h, list/l, i, x)
	return (h?.c.m().v++)
// Guards: a_scoped_chain_from_a_type_path_follows_the_declared_type_of_each_var (compile_proc.rs)
/datum/config
	var/datum/settings/settings
/datum/settings
	var/static/item_type = 5
var/datum/config/config
/proc/f1()
	return /datum/config::settings::item_type
/proc/f2()
	return /datum/config::settings.item_type
/proc/f3()
	return config::settings::item_type
// Guards: a_scoped_static_after_a_computed_value_is_read_alone (compile_proc.rs)
/datum/config
	var/datum/settings/settings
	var/static/own_type = 4
/datum/config/proc/get()
	return src
/datum/settings
	var/static/item_type = 5
var/datum/config/config
var/list/configs = list(null)
/proc/get_config() as /datum/config
	return config
/proc/get_untyped()
	return null
/proc/f1()
	return get_config()::settings::item_type
/proc/f2()
	return configs[1]:settings::item_type
/proc/f3()
	return get_untyped()::item_type
/proc/f4()
	return config.get()::own_type
/proc/f5()
	return config.get()::settings::item_type
/proc/f6()
	return get_config()?.settings::item_type
/proc/f7()
	return config.settings::item_type
