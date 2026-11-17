use super::{EXPORT_SAVE_DIALOGS, FileDialog, FilmcraftApp, Value, json, panels};

impl FilmcraftApp {
    // ---------------------------------------------------------------- files

    /// Destination of imports started from the currently shown Project panel view.
    pub fn import_bin(&self) -> filmcraft_project::BinId {
        panels::project::view_of(self, panels::project::shown_inst(self)).bin
    }

    pub fn file_dialog(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        match id {
            "file.import" => {
                let exts: Vec<&str> = filmcraft_media::VIDEO_EXTENSIONS
                    .iter()
                    .chain(filmcraft_media::AUDIO_EXTENSIONS)
                    .chain(filmcraft_media::STILL_EXTENSIONS)
                    .chain(&["srt", "vtt", "scc", "edl", "xml", "fcpxml", "otio", "aaf", "omf", "prfpset", "prproj"])
                    .copied()
                    .collect();
                let bin = params.get("bin").cloned();
                self.pick(FileDialog::open_files(&exts), move |app, paths| {
                    if paths.is_empty() {
                        return Ok(Value::Null);
                    }
                    let bin = bin.unwrap_or_else(|| json!(app.import_bin().0));
                    let r = app.session.execute("file.import", json!({"paths": paths, "bin": bin})).map_err(|e| e.to_string());
                    if let Ok(v) = &r {
                        if let Some(errs) = v.get("errors").and_then(Value::as_array)
                            && !errs.is_empty()
                        {
                            app.ui.status = errs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ");
                        } else if let Some(presets) = v.get("presets").and_then(Value::as_array)
                            && !presets.is_empty()
                        {
                            if !app.ui.expanded_fx.contains(&"Presets".to_string()) {
                                app.ui.expanded_fx.push("Presets".to_string());
                            }
                            app.ui.status = tlf!("Imported {n} preset(s)", n = presets.len());
                        }
                    }
                    r
                })
            }
            // File ▸ Import with Image Sequence: choose the first numbered still
            "file.importImageSequence" => self.pick(FileDialog::open_files(filmcraft_media::STILL_EXTENSIONS), |app, paths| {
                let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
                let r = app.session.execute("file.importImageSequence", json!({"path": path})).map_err(|e| e.to_string());
                if let Err(e) = &r {
                    app.ui.status = e.clone();
                }
                r
            }),
            "file.saveAs" | "file.save" | "file.saveCopy" => {
                let name = &self.session.project.name;
                let copy = id == "file.saveCopy";
                let cmd = if copy { "file.saveCopy" } else { "file.saveAs" };
                // a project from Premiere Pro saves as a Premiere Pro project unless told otherwise
                let dialog = if filmcraft_engine::interchange::premiere_save_mode(&self.session).is_some() {
                    let suggested = if copy { format!("{name} copy.prproj") } else { format!("{name}.prproj") };
                    FileDialog::save_as("Premiere Pro Project", &["prproj", "fcproj"], &suggested)
                } else {
                    FileDialog::SaveProject { name: if copy { format!("{name} copy.fcproj") } else { format!("{name}.fcproj") } }
                };
                self.pick(dialog, move |app, paths| {
                    let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
                    let r = app.session.execute(cmd, json!({"path": path})).map_err(|e| e.to_string());
                    match &r {
                        Ok(_) => crate::menus::saved_status(app, cmd, &path),
                        Err(e) => app.ui.status = e.clone(),
                    }
                    r
                })
            }
            "file.open" => self.pick(FileDialog::OpenProject, |app, paths| {
                let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
                if let Some(asked) = crate::panels::clip_dialogs::ask_before_open(app, &path) {
                    return asked;
                }
                app.session.execute("file.open", json!({"path": path})).map_err(|e| e.to_string())
            }),
            // a folder of Premiere projects (or the one with the layout kit)
            "file.learnPremiereLayouts" => self.pick(FileDialog::Folder { at: None }, |app, paths| {
                let Some(folder) = paths.into_iter().next() else { return Ok(Value::Null) };
                app.session.execute("file.learnPremiereLayouts", json!({"path": folder})).map_err(|e| e.to_string())
            }),
            "graphics.newFromFile" => {
                let exts: Vec<&str> = filmcraft_media::STILL_EXTENSIONS.iter().chain(filmcraft_media::VIDEO_EXTENSIONS).copied().collect();
                self.pick(FileDialog::open_files(&exts), |app, paths| {
                    let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
                    app.session.execute("graphics.newFromFile", json!({"path": path})).map_err(|e| e.to_string())
                })
            }
            "captions.import" => self.pick(FileDialog::open_files(&["srt", "vtt", "scc", "mcc", "stl", "ttml", "dfxp", "xml"]), |app, paths| {
                let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
                app.session.execute("captions.import", json!({"path": path})).map_err(|e| e.to_string())
            }),
            // File ▸ Export ▸ Premiere Pro Project…: next to the original, never over it by default
            "file.exportPrproj" => {
                let source = self
                    .session
                    .project
                    .items
                    .values()
                    .find_map(|i| i.metadata.get(filmcraft_engine::interchange::PREMIERE_SOURCE).cloned())
                    .unwrap_or_default();
                // a project from Premiere: beside its original under a new name; any other: its own name
                let suggested = match std::path::Path::new(&source).file_stem() {
                    Some(stem) if !source.is_empty() => format!("{} (FilmCraft).prproj", stem.to_string_lossy()),
                    _ => format!("{}.prproj", self.session.project.name),
                };
                let mut p = params.as_object().cloned().unwrap_or_default();
                self.pick(FileDialog::save_as("Premiere Pro Project", &["prproj"], &suggested), move |app, paths| {
                    let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
                    p.insert("path".into(), json!(path));
                    let r = app.session.execute("file.exportPrproj", Value::Object(p)).map_err(|e| e.to_string());
                    match &r {
                        Ok(v) => {
                            let notes: Vec<String> =
                                v["report"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                            app.ui.status = if notes.is_empty() { tlf!("Exported {path}", path) } else { format!("Exported {path}: {}", notes.join("; ")) };
                        }
                        Err(e) => app.ui.status = e.clone(),
                    }
                    r
                })
            }
            _ => match EXPORT_SAVE_DIALOGS.iter().find(|(c, ..)| *c == id) {
                Some(&(_, filter, ext)) => self.export_save_dialog(id, filter, ext, params),
                None => Err(format!("no dialog for {id}")),
            },
        }
    }

    /// File ▸ Export ▸ EDL…, Final Cut Pro XML… etc. from the menus (#382): ask where to save, then
    /// run the command with that `path` and the rest of `params`.
    fn export_save_dialog(&mut self, id: &str, filter: &str, ext: &str, params: &Value) -> Result<Value, String> {
        filmcraft_engine::find_command(id).map_or(Ok(()), |c| (c.enabled)(&self.session))?;
        if !FileDialog::save_as(filter, &[ext], "").available(&self.hooks) {
            return Err("no save dialog available: run the command with a `path`".into());
        }
        // Timelines are named after the sequence; a selection must not suggest the open project's file.
        let sequence = self.session.state.active_sequence.and_then(|s| self.session.project.item(s)).map(|i| i.name.clone());
        let stem = match id {
            "file.exportAle" => self.session.project.name.clone(),
            "file.exportSelectionProject" => format!("{} selection", self.session.project.name),
            _ => sequence.unwrap_or_else(|| self.session.project.name.clone()),
        };
        let stem: String = stem.chars().map(|c| if matches!(c, '/' | '\\') || c.is_control() { '-' } else { c }).collect();
        let stem = if stem.trim().is_empty() { "Untitled".to_string() } else { stem };
        let mut p = params.as_object().cloned().unwrap_or_default();
        let id = id.to_string();
        self.pick(FileDialog::save_as(filter, &[ext], &format!("{stem}.{ext}")), move |app, paths| {
            let Some(path) = paths.into_iter().next() else { return Ok(Value::Null) };
            p.insert("path".into(), json!(path));
            let r = app.session.execute(&id, Value::Object(p)).map_err(|e| e.to_string());
            app.ui.status = match &r {
                Ok(_) => tlf!("Exported {path}", path),
                Err(e) => e.clone(),
            };
            r
        })
    }
}
