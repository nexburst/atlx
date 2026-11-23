//! The records Confluence commands return (see [`crate::schema`]).
//!
//! The API functions in [`crate::confluence`] return REST v2 objects with a
//! few computed keys added (`type`, `externalUrl`, a rendered `body`); these
//! records pick the fields worth printing from them.

use crate::schema::{Budget, Field, Record, Source, truncate::BODY};
use serde_json::Value;

/// The ID of the page or blog post a comment, attachment or task is on.
const CONTAINER: Source = Source::First(&["pageId", "blogPostId", "contentId"]);

/// A page or blog post as `page get` prints it: metadata and the body.
pub const PAGE: Record = Record {
    name: "confluence.page",
    description: "a Confluence page or blog post, with its body",
    primary: "id",
    fields: &[
        Field::new("id", "string", "content ID", "id"),
        Field::new("type", "string", "page or blogpost", "type"),
        Field::new("title", "string", "title", "title"),
        Field::new("status", "string", "current, draft, archived or trashed", "status"),
        Field::new("spaceId", "string", "ID of the space it is in", "spaceId"),
        Field::new("parentId", "string", "parent page ID (pages only)", "parentId"),
        Field::new(
            "version",
            "integer",
            "version number; pass it as --if-version when you update",
            "version.number",
        ),
        Field::new("updated", "datetime", "when this version was made", "version.createdAt"),
        Field::new("url", "string", "web address", "externalUrl"),
        Field::new(
            "body",
            "markdown",
            "the body in --body-format (Markdown by default); content Markdown cannot show is marked <!-- atlx: ... --> and listed in a lossy_body warning",
            "body",
        )
        .truncated(BODY),
        Field::new(
            "outline",
            "object[]",
            "with --outline: the headings, each {level, text, anchor, localId, wordCount}",
            "outline",
        ),
        Field::new("author", "string", "account ID of the version's author", "version.authorId")
            .full(),
        Field::new("created", "datetime", "when it was created", "createdAt").full(),
        Field::new("ownerId", "string", "account ID of its owner", "ownerId").full(),
        Field::new("bodyFormat", "string", "markdown, adf or storage", "bodyFormat").full(),
        Field::new(
            "versionMessage",
            "string",
            "the message of this version",
            "version.message",
        )
        .full(),
    ],
};

/// A page or blog post in a list or a write result: metadata only.
pub const PAGE_ITEM: Record = Record {
    name: "confluence.page-item",
    description: "a Confluence page or blog post without its body",
    primary: "id",
    fields: &[
        Field::new("id", "string", "content ID", "id"),
        Field::new("type", "string", "page or blogpost", "type"),
        Field::new("title", "string", "title", "title"),
        Field::new(
            "status",
            "string",
            "current, draft, archived or trashed",
            "status",
        ),
        Field::new("spaceId", "string", "ID of the space it is in", "spaceId"),
        Field::new(
            "parentId",
            "string",
            "parent page ID (pages only)",
            "parentId",
        ),
        Field::new("version", "integer", "version number", "version.number"),
        Field::new(
            "updated",
            "datetime",
            "when this version was made",
            "version.createdAt",
        ),
        Field::new("url", "string", "web address", "externalUrl"),
        Field::new(
            "author",
            "string",
            "account ID of the version's author",
            "version.authorId",
        )
        .full(),
        Field::new("created", "datetime", "when it was created", "createdAt").full(),
        Field::new("ownerId", "string", "account ID of its owner", "ownerId").full(),
    ],
};

pub const COMMENT: Record = Record {
    name: "confluence.comment",
    description: "a footer or inline comment",
    primary: "id",
    fields: &[
        Field::new("id", "string", "comment ID", "id"),
        Field::new("type", "string", "footer or inline", "commentType"),
        Field::new("pageId", "string", "page or blog post it is on", "").source(CONTAINER),
        Field::new(
            "parentId",
            "string",
            "the comment it replies to",
            "parentCommentId",
        ),
        Field::new(
            "author",
            "string",
            "account ID of the author",
            "version.authorId",
        ),
        Field::new(
            "created",
            "datetime",
            "when this version was written",
            "version.createdAt",
        ),
        Field::new("version", "integer", "version number", "version.number"),
        Field::new(
            "resolution",
            "string",
            "inline comments: open, resolved, reopened or dangling",
            "resolutionStatus",
        ),
        Field::new("body", "markdown", "the body in --body-format", "body").truncated(BODY),
        Field::new(
            "replies",
            "object[]",
            "with --replies: the replies, nested",
            "replies",
        ),
        Field::new("url", "string", "web address", "externalUrl"),
        Field::new("status", "string", "current or draft", "status").full(),
        Field::new(
            "selection",
            "string",
            "inline comments: the text it is anchored to",
            "properties.inlineOriginalSelection",
        )
        .full(),
        Field::new(
            "repliesTruncated",
            "boolean",
            "the reply thread was cut at 500 replies",
            "repliesTruncated",
        )
        .full(),
    ],
};

pub const ATTACHMENT: Record = Record {
    name: "confluence.attachment",
    description: "a file attached to a page or blog post",
    primary: "id",
    fields: &[
        Field::new("id", "string", "attachment ID (att...)", "id"),
        Field::new("title", "string", "file name", "title"),
        Field::new("mediaType", "string", "media type", "mediaType"),
        Field::new("fileSize", "integer", "size in bytes", "fileSize"),
        Field::new("version", "integer", "version number", "")
            .source(Source::First(&["version.number", "version"])),
        Field::new("pageId", "string", "page or blog post it is on", "").source(CONTAINER),
        Field::new("comment", "string", "comment of this version", "comment"),
        Field::new("url", "string", "web address", "externalUrl"),
        Field::new("created", "datetime", "when it was attached", "createdAt").full(),
        Field::new("downloadLink", "string", "download path", "downloadLink").full(),
        Field::new(
            "action",
            "string",
            "upload: created, or updated (a new version of a file with the same name)",
            "action",
        )
        .full(),
    ],
};

pub const LABEL: Record = Record {
    name: "confluence.label",
    description: "a label on a page or blog post",
    primary: "name",
    fields: &[
        Field::new("name", "string", "label name", "name"),
        Field::new("prefix", "string", "global, my or team", "prefix"),
        Field::new("id", "string", "label ID", "id"),
    ],
};

pub const VERSION: Record = Record {
    name: "confluence.version",
    description: "one version of a page or blog post",
    primary: "number",
    fields: &[
        Field::new("number", "integer", "version number", "number"),
        Field::new("message", "string", "version message", "message"),
        Field::new(
            "minorEdit",
            "boolean",
            "marked as a minor edit",
            "minorEdit",
        ),
        Field::new("author", "string", "account ID of the author", "authorId"),
        Field::new("created", "datetime", "when it was made", "createdAt"),
        Field::new(
            "title",
            "string",
            "version get: the title at that version",
            "title",
        ),
        Field::new(
            "body",
            "markdown",
            "version get: the body at that version, in --body-format",
            "body",
        )
        .truncated(BODY),
    ],
};

fn description(v: &Value) -> Value {
    v["description"]["plain"]["value"].clone()
}

pub const SPACE: Record = Record {
    name: "confluence.space",
    description: "a Confluence space",
    primary: "key",
    fields: &[
        Field::new("id", "string", "space ID", "id"),
        Field::new("key", "string", "space key", "key"),
        Field::new("name", "string", "name", "name"),
        Field::new("type", "string", "global or personal", "type"),
        Field::new("status", "string", "current or archived", "status"),
        Field::new("homepageId", "string", "ID of its home page", "homepageId"),
        Field::new("url", "string", "web address", "externalUrl"),
        Field::new("description", "string", "plain-text description", "")
            .source(Source::Fn(description))
            .full(),
        Field::new("created", "datetime", "when it was created", "createdAt").full(),
        Field::new(
            "authorId",
            "string",
            "account ID of its creator",
            "authorId",
        )
        .full(),
        Field::new(
            "labels",
            "object[]",
            "with --labels: the space's labels as {id, name, prefix}",
            "labels",
        )
        .full(),
    ],
};

pub const TEMPLATE: Record = Record {
    name: "confluence.template",
    description: "a page template or blueprint",
    primary: "id",
    fields: &[
        Field::new("id", "string", "template ID", "templateId"),
        Field::new("name", "string", "name", "name"),
        Field::new("description", "string", "description", "description"),
        Field::new("type", "string", "page or blueprint", "templateType"),
        Field::new(
            "spaceKey",
            "string",
            "space it belongs to (none: global)",
            "space.key",
        ),
        Field::new(
            "body",
            "markdown",
            "template get: the body in --body-format",
            "body",
        )
        .truncated(BODY),
    ],
};

pub const SEARCH_HIT: Record = Record {
    name: "confluence.search-hit",
    description: "one CQL search result",
    primary: "id",
    fields: &[
        Field::new("id", "string", "content ID", "content.id"),
        Field::new(
            "type",
            "string",
            "page, blogpost, comment, attachment, ...",
            "",
        )
        .source(Source::First(&["content.type", "entityType"])),
        Field::new(
            "title",
            "string",
            "title, without highlight markers",
            "title",
        ),
        Field::new(
            "space",
            "string",
            "name of the space it is in",
            "resultGlobalContainer.title",
        ),
        Field::new(
            "excerpt",
            "string",
            "matched text, without highlight markers",
            "excerpt",
        ),
        Field::new("updated", "datetime", "last modified", "lastModified"),
        Field::new("url", "string", "web address", "externalUrl"),
        Field::new("status", "string", "content status", "content.status").full(),
    ],
};

pub const TASK: Record = Record {
    name: "confluence.task",
    description: "a task (checkbox) written in a page or blog post",
    primary: "id",
    fields: &[
        Field::new("id", "string", "task ID", "id"),
        Field::new("status", "string", "complete or incomplete", "status"),
        Field::new("pageId", "string", "page or blog post it is in", "").source(CONTAINER),
        Field::new("spaceId", "string", "space it is in", "spaceId"),
        Field::new(
            "assignee",
            "string",
            "account ID it is assigned to",
            "assignedTo",
        ),
        Field::new("due", "datetime", "due date", "dueAt"),
        Field::new("body", "markdown", "the task text", "body")
            .truncated(Budget::chars(2_000, "body-offset")),
        Field::new(
            "creator",
            "string",
            "account ID of its creator",
            "createdBy",
        )
        .full(),
        Field::new("created", "datetime", "when it was created", "createdAt").full(),
        Field::new(
            "completed",
            "datetime",
            "when it was completed",
            "completedAt",
        )
        .full(),
        Field::new(
            "completedBy",
            "string",
            "account ID that completed it",
            "completedBy",
        )
        .full(),
    ],
};

fn operation(v: &Value) -> Value {
    serde_json::json!({
        "key": v["operation"]["key"],
        "targetType": v["operation"]["targetType"],
    })
}

pub const PERMISSION: Record = Record {
    name: "confluence.space-permission",
    description: "one permission granted in a space",
    primary: "id",
    fields: &[
        Field::new("id", "string", "permission ID", "id"),
        Field::new(
            "principal",
            "object",
            "who has it: {type: user|group|role, id}",
            "principal",
        ),
        Field::new(
            "operation",
            "object",
            "what it allows: {key, targetType}",
            "",
        )
        .source(Source::Fn(operation)),
    ],
};
