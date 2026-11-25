/proc/lib_tool(n)
	return n * 2 // @lib_tool

/proc/sc_tools()
	var/doubled = lib_tool(2)
	var/more = sub_tool(doubled)
	world.log << "dap: tools [more]"

/proc/sc_declares()
	world.log << "dap: declares [sub_declares(1)]"
