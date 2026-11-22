// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 5359, "515.1637: new: globals differ"
#define DEBUG

#define FZD3 1

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
	var/static/list/vl9 = list()
	var/va10 = 0.001
	var/datum/fz/s4/vo11
	var/tmp/vn12 = 0x1F
	var/list/vl13
	var/static/va14
	var/vn15 = 0

/datum/fz/s4
	va10 = "  pad  "

/obj/fz
	var/vn16 = 5
	var/static/vn17 = 3
	var/const/vn18 = 7

/mob/fz
	var/list/vl21
	var/datum/fz/vo22

/mob/fz/s6
	desc = "desc 27"

var/gt28 = "abc"
var/ga29 = null
var/ga30
var/gn31 = 0.001

/proc/fz_setup()

/proc/fz_fields(datum/d)
	return ""

/proc/fz_globals()
	return "news=[fz_news] gt28=[fz_show(gt28, 2)] ga29=[fz_show(ga29, 2)] ga30=[fz_show(ga30, 2)] gn31=[fz_show(gn31, 2)]"

/proc/fz_intro()
	fz_news = 0

/proc/fz_run()
	fz_case(158, /obj/fz/s5, "t149", 1)

/proc/gh39()

/proc/gh41()
	var/loops = 0
	var/n1 = -3.25
	var/s1 = ""
	var/list/L = list(0, "12abc")
	if (length(L))
	switch ((!ga30))
		if (100)
			for (var/e42 as anything in L)
				if (++loops > 40) break
				L["zz"] += (~gn31)
		if ("x")
			try
				n1 ||= isinf((loops * 0 + (7 + gn31)))
			catch

/proc/gh62()
	var/n1 = -4
	var/s1 = "1"
	var/list/L = list(-1, "x")
	switch (@"raw\n[x]")
		if (0, "Hello")
		if (3)
			if (length(L))
				L[length(L)] = world.tick_lag
		else
			n1 = n1

/obj/fz/proc/h73(x1, x2)
	var/s1 = ""
	var/list/L = list(10, "12abc")
	var/v
	if (((args.Copy() ? "a\tb" : "UPPER lower") + "[0.001]"))
	return list(list(1), list())

/obj/fz/s5/proc/h78()
	var/loops = 0
	var/n1 = 255
	var/s1 = "k=1&j=2"
	var/list/L = list(0.5, "x")
	try
		if (text2path("/obj/fz"))
		else
	catch (var/x79)
	for (var/xd81 in 1 to 2)
		if ((min(max((length(L) < 5), -16777216), 16777215) >> 1))
			if (!(FZD3))
				if (!(ga30))

/mob/fz/proc/h88(x1)
	var/list/L = list(0.001, "Hello")
	var/v
	L -= x1
	return (v && (list("  pad  ", (~x1), "n[(65535 != L.len)]")[1] && usr))

/obj/fz/proc/t149(a, b = 2, c)
	var/loops = 0
	var/n1 = 255
	var/s1 = "12abc"
	var/n2 = 0
	var/list/L = list(65535, "UPPER lower", 3)
	var/list/M = list("a" = 0x1F, "b" = "Hello")
	var/mob/fz/o = new /mob/fz
	var/datum/fz/s4/p = new /datum/fz/s4
	var/v
	L["k"] -= (-1 >= p)
	vn17 *= (~(vn16 + p.vn12))
	return list(a, b, c, n1, n2, s1, L, M, o, p, v, ., loops)

/obj/fz/s5/t149(a, b = 2, c)
	var/loops = 0
	var/n1 = 1
	var/s1 = "q\"q"
	var/n2 = 2
	var/list/L = list(256, "1", 5)
	var/list/M = list("a" = 10, "b" = "UPPER lower")
	var/mob/fz/o = new /mob/fz
	var/datum/fz/s4/p = new /datum/fz/s4(0.5, "A,b,c")
	var/v
	while ("[o.h88()]")
		if (++loops > 40) break
		++p.vn12
	gh39()
	gn31 ^= min(max(arctan(16777215), -16777216), 16777215)
	while (7)
		if (++loops > 40) break
