// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 986, "515.1647: old: runtime error only here: type mismatch: cannot compare "…" to N"
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
	var/vn17 = 100

var/gn25
var/gn27 = 7

/proc/fz_setup()
	fz_old["/obj/fz/s4"] = new /obj/fz/s4

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(73, /obj/fz/s4, "Verb 65", 1)

/obj/fz/proc/h45(x1)

/obj/fz/s4/verb/vb64(a as num, b as num, c as text)
	set name = "Verb 65"
	var/n1 = 100
	var/list/L = list(-1, "  pad  ", 2.5)
	var/obj/fz/s4/p = new /obj/fz/s4(null, 0.5, "abc")
	if (length(L))
		L[1] = p.h45(x1 = gn27)
	if ((n1 = global.gn25))
		vn17 |= min(max(max(L), -16777216), 16777215)
	var/p67 = &gn25
	*p67 = 2.5
