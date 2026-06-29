-- Resolve the absolute output path of an xmake binary target.
--
-- Usage: xmake l targetpath.lua <targetname> [projectdir]
--
-- When <projectdir> is given, the script changes into that directory first so
-- the command can be launched from anywhere (the Zed extension process API has
-- no notion of a working directory).
--
-- Output is delimited by `__begin__` / `__end__` markers so callers can ignore
-- any warnings xmake writes to stdout before the real path.

import("core.project.config")
import("core.project.project")

function main(targetname, projectdir)
    -- Optionally move into the project directory.
    if projectdir and os.isdir(projectdir) then
        os.cd(projectdir)
    end

    -- Load the project configuration (requires an xmake.lua to be present).
    if not os.isfile(os.projectfile()) then
        return
    end
    config.load()

    -- Pick the target: the one named by the caller, otherwise the first
    -- default binary target found in the project.
    local target = nil
    if targetname and #targetname > 0 then
        target = project.target(targetname)
    end
    if not target then
        for _, t in pairs(project.targets()) do
            local default = t:get("default")
            if (default == nil or default == true) and t:get("kind") == "binary" then
                target = t
                break
            end
        end
    end

    -- Start marker: ignore anything logged to stdout before this point.
    print("__begin__")

    if target then
        local targetfile = target:targetfile()
        if not path.is_absolute(targetfile) then
            targetfile = path.absolute(targetfile, os.projectdir())
        end
        print(targetfile)
    end

    -- End marker: ignore deprecation/warnings emitted afterwards.
    print("__end__")
end
