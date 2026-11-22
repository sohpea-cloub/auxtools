// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 39, "new: result differs"
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

/datum/fz/s3/s4

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(123, /obj/fz/s6, "t121", 1)

/obj/fz/s6/proc/t121(a, b = 2, c)
	var/loops = 0
	var/n1 = 0
	var/s1 = "Hello"
	var/n2 = 2
	var/list/L = list(16777215, "", 100)
	var/list/M = list("a" = 2.5, "b" = "abc")
	var/datum/fz/s3/s4/o = new /datum/fz/s3/s4(-4, "a\tb")
	var/datum/fz/p = new /datum/fz
	var/v
	n2 = length_char("xéyéz")
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
