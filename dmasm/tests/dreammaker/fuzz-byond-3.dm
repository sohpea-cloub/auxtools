// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 3, "515.1647: new: runtime error only here: type mismatch: "…" ** -N"
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

/obj/fz
	var/vn21 = 3

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(86, /datum/fz/s5, "t77", 1)

/proc/gh39(x1 = null, x2 = null, x3)

/obj/fz/s7/proc/h42(x1, x2, x3)

/datum/fz/s5/proc/h45(x1, x2)

/datum/fz/s5/proc/h47(x1 = "abc", x2)
	var/list/L = list(-3.25, "1")
	gh39(1 %% x1 ** -1, (L + args.Copy()), findtext("k=1&j=2", "", 2))

/datum/fz/proc/t77(a, b = 2, c)

/datum/fz/s5/t77(a, b = 2, c)
	var/datum/fz/s5/o = new /datum/fz/s5(16777215, "3.5")
	var/obj/fz/s7/p = new /obj/fz/s7(null, 0)
	h45(p.vn21, call(o, /datum/fz/s5/proc/h47)())
