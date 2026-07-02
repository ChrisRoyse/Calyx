# codebase-memory MCP — agent reference

project id: `C-code-Calyx-Dev` (required arg on every call). Rust workspace, `full` index, ~53k nodes/257k edges.
Snapshot, not live: re-run `detect_changes` (cheap) or `index_repository` (full) after edits. Use graph tools before grep for code structure; grep/Read for text/config/non-code.

## tools (signature → use)
- `list_projects()` → indexed? id? (`[]` if none)
- `index_repository(repo_path, mode=full|moderate|fast|cross-repo-intelligence, persistence?, target_projects?)` → build/refresh. full=+SIMILAR_TO/semantic (needed for semantic_query); fast=no semantic; cross-repo needs target_projects=["*"]. persistence writes `.codebase-memory/graph.db.zst`.
- `index_status(project)` → build progress/health
- `delete_project(project)` → drop index (rebuild after)
- `get_graph_schema(project)` → labels/edges/counts. **Run before Cypher** — live edges differ from generic docs.
- `get_architecture(project, aspects=[...])` → packages + Leiden module clusters. aspects=["all"] is ~67k chars here → narrow aspects or run in subagent.
- `search_graph(project, query|name_pattern|semantic_query, label?, file_pattern?, qn_pattern?, min_degree?, max_degree?, relationship?, direction?, exclude_entry_points?, limit=200, offset)` → find symbols. Paginate: check `has_more`/`total`, offset+=limit.
- `search_code(project, pattern, regex?, file_pattern?, path_filter?, mode=compact|full|files, limit=10, context?)` → grep enriched+ranked by structure. No offset (raise limit / narrow).
- `get_code_snippet(project, qualified_name, include_neighbors?)` → exact source. Get qn from search_graph first.
- `trace_path(project, function_name, direction=inbound|outbound|both, depth=3, mode=calls|data_flow|cross_service, parameter_name?, risk_labels?, include_tests?)` → relationships. Needs exact name.
- `query_graph(project, query, max_rows?)` → Cypher. Ceiling 100k rows, no offset → LIMIT in query.
- `detect_changes(project, base_branch=main, since?, depth=2, scope?)` → git diff → impacted symbols.
- `manage_adr(project, mode=get|sections|update, sections?, content?)` → persist design decisions. This repo: none yet.
- `ingest_traces(project, traces=[...])` → enrich graph w/ runtime call data. Optional.

## search_graph modes (independent, combinable)
- `query="..."` → BM25 NL/keyword; camelCase+snake split; Fn/Method+10, Route+8, Class+5. Best discovery. Overrides name_pattern.
- `name_pattern=".*re.*"` → regex on name.
- `semantic_query=["k1","k2"]` → **array**, vector cosine, bridges vocab (find "publish" via "send"). Needs full/moderate. Lands in `semantic_results` field.

degree recipes: dead code `max_degree=0, exclude_entry_points=true` · fan-out `min_degree=10, relationship="CALLS", direction="outbound"` · fan-in (blast radius) same w/ `direction="inbound"`.

## live schema (confirm w/ get_graph_schema after re-index)
nodes: Function 16.3k, Field 12k, Section 6.6k, Method 5.5k, Variable 3.7k, Class 2.9k, Module 2.6k, File 2.6k, Enum 369, Folder 262, Decorator 212, Interface 115, Type 89, Route 63, Macro 1.
edges: USAGE 101k, CALLS 76k, DEFINES 50k, IMPORTS 7.2k, DECORATES 6.7k, DEFINES_METHOD 5.3k, SIMILAR_TO 3.2k, WRITES 2.9k, CONTAINS_FILE 2.6k, FILE_CHANGES_WITH 905, IMPLEMENTS 484, CONTAINS_FOLDER 453, SEMANTICALLY_RELATED 57, HTTP_CALLS 50, TESTS 32, CONFIGURES 21, RAISES 1.
(no HANDLES/OVERRIDE/ASYNC_CALLS here despite generic docs.)

Function/Method props (queryable in Cypher): complexity(cyclomatic), cognitive, loop_count, loop_depth, transitive_loop_depth(interprocedural, propagated on CALLS), linear_scan_in_loop(hidden O(n²)), alloc_in_loop, recursive/self_recursive/recursion_in_loop, unguarded_recursion, param_count, max_access_depth, is_entry_point/is_exported/is_test.

## Cypher recipes
```cypher
MATCH (a)-[:CALLS]->(b:Function {name:'build_report'}) RETURN a.qualified_name        // callers
MATCH (a)-[r:CALLS]->(b) RETURN a.name,b.name,r.confidence,r.strategy LIMIT 25         // edges+props
MATCH (r:Route) RETURN r.method,r.name,r.file_path                                     // http routes
MATCH (a)-[r:FILE_CHANGES_WITH]->(b) RETURN a.name,b.name,r.coupling_score
  ORDER BY r.coupling_score DESC LIMIT 20                                              // co-change coupling
MATCH (f:Function) WHERE f.transitive_loop_depth>=3 OR f.linear_scan_in_loop>=1
  RETURN f.qualified_name,f.transitive_loop_depth,f.linear_scan_in_loop
  ORDER BY f.transitive_loop_depth DESC LIMIT 30                                       // hot paths
MATCH (f:Function) WHERE f.unguarded_recursion=true RETURN f.qualified_name            // recursion risk
MATCH (f:Function) WHERE f.complexity>=15 RETURN f.qualified_name,f.complexity,f.cognitive
  ORDER BY f.complexity DESC LIMIT 25                                                  // refactor targets
```

## gotchas
1. `get_graph_schema` before Cypher — trust live edges, not generic list.
2. `search_graph(relationship=)` filters NODES by degree, not edges → use Cypher for edges+props.
3. `query_graph` 100k cap, no offset → LIMIT in query; browse large sets via search_graph pagination.
4. `trace_path` exact names only (search_graph first); `direction=both` or miss cross-service callers.
5. `search_graph` truncates at limit silently → page via has_more/offset. `search_code` no offset → raise limit/narrow.
6. index is a snapshot → detect_changes/index_repository after code changes.
7. `get_architecture(["all"])` too big to inline → narrow or subagent.

## recipe table
| ask | call |
|-----|------|
| indexed? | `list_projects()` |
| find by concept | `search_graph(project, query=)` |
| find by name | `search_graph(project, name_pattern=, label="Function")` |
| read source | search_graph → `get_code_snippet(project, qualified_name=)` |
| who calls X | `trace_path(project, function_name=X, direction="inbound")` |
| what X calls | `trace_path(..., direction="outbound")` |
| full context | `trace_path(..., direction="both")` |
| branch impact | `detect_changes(project, base_branch="main")` |
| dead code | `search_graph(project, max_degree=0, exclude_entry_points=true)` |
| blast-radius hubs | `search_graph(project, min_degree=10, relationship="CALLS", direction="inbound")` |
| hot paths/O(n²) | query_graph `WHERE f.transitive_loop_depth>=3 OR f.linear_scan_in_loop>=1` |
| co-changing files | query_graph `FILE_CHANGES_WITH` by `coupling_score` |
| persist decision | `manage_adr(project, mode="update", content=)` |
