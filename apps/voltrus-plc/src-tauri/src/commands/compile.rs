use crate::state::AppState;
use mt_plc_types::CompiledModule;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CompileResult {
    pub success: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub pou_count: usize,
    pub variable_count: usize,
    pub module: Option<CompiledModule>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Diagnostic {
    pub severity: String,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub source: String,
}

#[tauri::command]
pub async fn compile_source(
    state: tauri::State<'_, AppState>,
    source: String,
    file_name: String,
    extra_sources: Option<Vec<(String, String)>>,
) -> Result<CompileResult, String> {
    let result = if let Some(ref extras) = extra_sources {
        let mut all_sources = vec![(file_name.clone(), source.clone())];
        all_sources.extend(extras.iter().cloned());
        // Reorder: main source (program) first
        if all_sources.len() > 1 {
            let main = all_sources.remove(0);
            all_sources.insert(0, main);
        }
        mt_stc::compile_project(&all_sources)
    } else {
        mt_stc::compile(&source)
    };

    let diagnostics = result
        .diagnostics
        .iter()
        .map(|d| Diagnostic {
            severity: d.severity.clone(),
            message: d.message.clone(),
            line: d.line,
            col: d.col,
            end_line: d.end_line,
            end_col: d.end_col,
            source: d.source.clone(),
        })
        .collect::<Vec<_>>();

    let compile_result = CompileResult {
        success: result.success,
        pou_count: result.pou_count,
        variable_count: result.variable_count,
        module: result.module,
        diagnostics: diagnostics.clone(),
    };

    let mut compiler = state.compiler.lock().await;
    compiler.last_diagnostics = diagnostics;
    if let Some(ref module) = compile_result.module {
        compiler.last_module = Some(module.clone());
    }

    Ok(compile_result)
}

#[tauri::command]
pub async fn get_diagnostics(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Diagnostic>, String> {
    let compiler = state.compiler.lock().await;
    Ok(compiler.last_diagnostics.clone())
}
