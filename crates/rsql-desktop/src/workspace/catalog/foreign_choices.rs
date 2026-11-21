use super::*;
impl Workspace {
    pub(in crate::workspace) fn load_fk_schemas(&mut self, project: &str, cx: &mut Context<Self>) {
        self.load_node(project, Node::Schemas, cx);
    }
    pub(in crate::workspace) fn load_fk_tables(
        &mut self,
        project: &str,
        schema: &str,
        cx: &mut Context<Self>,
    ) {
        self.load_node(project, Node::Section(schema.into(), Section::Tables), cx);
    }
    pub(in crate::workspace) fn fk_schemas(&self, project: &str) -> Result<&[String], &str> {
        match self
            .navigation
            .catalogs
            .get(project)
            .and_then(|c| c.entries.get(&Node::Schemas))
        {
            Some(Load::Ready(Data::Names(names))) => Ok(names),
            Some(Load::Failed(err)) => Err(err),
            _ => Err("Loading schemas…"),
        }
    }
    pub(in crate::workspace) fn fk_tables(
        &self,
        project: &str,
        schema: &str,
    ) -> Result<impl Iterator<Item = &ObjectRef>, &str> {
        match self.navigation.catalogs.get(project).and_then(|c| {
            c.entries
                .get(&Node::Section(schema.into(), Section::Tables))
        }) {
            Some(Load::Ready(Data::Relations(rows))) => Ok(rows.iter().map(|r| &r.object)),
            Some(Load::Failed(err)) => Err(err),
            _ => Err("Loading tables…"),
        }
    }
}
