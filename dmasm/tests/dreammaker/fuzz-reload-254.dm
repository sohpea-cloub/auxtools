// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 254, "new: result differs"
// Builds: 516.1648-
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
	var/const/vn8 = 5
	var/vt9 = "A,b,c"
	var/static/list/vl10 = list("a" = 1, "b" = 2)
	var/va11 = null
	var/tmp/datum/fz/s3/s6/vo12
	var/tmp/vn13 = 0

/datum/fz/s3/s4/s5
	vn13 = 2

/datum/fz/s3/s6
	parent_type = /datum/fz/s3/s4/s5

var/gn27 = gn28 + 1
var/gn28 = -3.25

/proc/fz_setup()

/proc/fz_fields(datum/d)
	if (d.type == /datum/fz/s3)
		var/datum/fz/s3/x1 = d
		return "vn8=[fz_show(x1.vn8, 2)] vt9=[fz_show(x1.vt9, 2)] vl10=[fz_show(x1.vl10, 2)] va11=[fz_show(x1.va11, 2)] vo12=[fz_show(x1.vo12, 2)] vn13=[fz_show(x1.vn13, 2)]"

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(182, /mob/fz, "t176", 1)

/proc/gh53(x1)
	gn27 = 7
	return gn27--

/mob/fz/proc/t176(a, b = 2, c)
	var/loops = 0
	var/n1 = 2.5
	var/s1 = "k=1&j=2"
	var/n2 = 65535
	var/list/L = list(0x1F, "a b", 2)
	var/list/M = list("a" = 5, "b" = "k=1&j=2")
	var/datum/fz/s3/o = new /datum/fz/s3(-3.25, "A,b,c")
	var/datum/fz/s3/p = new(0x1F, "  pad  ")
	var/v
	p.vn13 = 256
	for (var/i179 = 1 to gh53((~global.gn28)) step 0.5)
		for (var/k181, w181 in M)
			p.va11 = (p.vn13 ? /datum/fz/s3/s6::vn13 : gn28)
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)
