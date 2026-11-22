// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 15, "516.1659: new: runtime error only here: Cannot create objects of type null."
#define DEBUG

var/fz_news = 0
var/fz_budget = 0
var/list/fz_out = list()
var/list/fz_logs = list()
var/list/fz_old = list()
/proc/fz_show(v, depth = 0)
/proc/fz_log(tag, v)
/proc/fz_line(t)
/proc/fz_sort(list/l)
/proc/fz_call(id, dom, datum/d, name)
/proc/fz_case(id, t, name, old)
/proc/fz_ti(t, list/names, list/procs, datum/old)
/proc/fz_main()
/world/New()

/datum/fz
	var/vn9 = 10
	var/tmp/vt15 = "3.5"

/obj/fz/s8/New(loc, n, t)

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(110, /datum/fz/s4/s5, "t100", 1)

/proc/gh46(x1, x2)
	var/s1 = "x"
	var/list/L = list(0.001, "k=1&j=2")
	return list("zz" = args.Copy(), "b" = (L ~! new /obj/fz/s8(null, isinf(/datum/fz/s4/s7::vn9), ("x" || s1))))

/datum/fz/proc/t76(a, b = 2, c)
	var/list/L = list(16777215, "q\"q", 255)
	try
		if (length(L)) throw EXCEPTION("boom 80")
	catch (var/x80)

/datum/fz/s4/s7/t76(a, b = 2, c)

/datum/fz/proc/t100(a, b = 2, c)

/datum/fz/s4/s5/t100(a, b = 2, c)
	var/list/L = list(65535, "UPPER lower", 0.001)
	var/datum/fz/s4/o = new /datum/fz/s4
	var/v
	. = call(/proc/gh46)((L - o.vt15), (!v))
	try
		text2file("q\"q", "fzf109.txt")
	catch (var/x108)
