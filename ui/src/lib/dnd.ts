// Arrastrar y soltar en el árbol: durante `dragover` el navegador no deja leer los datos, así que
// el elemento arrastrado se guarda aquí.
export const drag: { current: { collectionId: string; itemId: string } | null } = { current: null };

export const DND_TYPE = 'application/x-runpost-item';
