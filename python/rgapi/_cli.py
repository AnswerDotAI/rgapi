"Command-line access to notebook-aware search."

from fastcore.script import call_parse
from . import nbrg


@call_parse(pos=['root'])
def nbrg_cli(
    pattern:str, # Regex pattern to search for
    root:str='.', # File or directory to search
    cell_context:int=0, # Neighbouring cells to include before and after matches
    multiline:bool=False, # Allow matches across lines within a cell?
    smart_case:bool=False, # Use case-sensitive matching when the pattern contains uppercase?
    case:bool=False, # Force case-sensitive matching?
    paths:bool=False, # Return only matching notebook paths?
    count:bool=False, # Return the number of matching cells?
    max_results:int=None, # Maximum matching cells to return
    maxlen:int=180, # Maximum source characters displayed per cell
    glob:str=None, # Glob selecting notebook paths
    exclude:str=None, # Glob excluding notebook paths
    hidden:bool=False, # Search hidden files and directories?
    max_depth:int=None, # Maximum directory depth to search
    timeout_ms:int=None, # Stop searching after this many milliseconds
):
    "Search notebook cell sources and report stable cell IDs."
    print(nbrg(pattern, root, cell_context=cell_context, multiline=multiline, smart_case=smart_case,
        case_sensitive=True if case else None, paths=paths, count=count, max_results=max_results, maxlen=maxlen,
        glob=glob, exclude=exclude, hidden=hidden, max_depth=max_depth, timeout_ms=timeout_ms))
